use ical::{parser::ical::component::IcalEvent, property::Property};
use miette::*;
use time::{
    format_description::well_known::iso8601, Date, OffsetDateTime, PrimitiveDateTime, Time,
    UtcOffset, Weekday,
};

const ICAL_DT: iso8601::Config = iso8601::Config::DEFAULT
    .set_use_separators(false)
    .set_time_precision(iso8601::TimePrecision::Second {
        decimal_digits: None,
    });

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub summary: String,
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
}

impl Event {
    pub fn covers(&self, date: Date) -> bool {
        self.start.date() <= date && self.end.date() >= date
    }
}

pub type Calendar = Vec<Event>;

/// The returned calendar is sorted by start date.
pub fn parse_ical(data: &str, offset: UtcOffset, limit: OffsetDateTime) -> Result<Calendar> {
    let parser = ical::IcalParser::new(data.as_bytes());
    let mut evs = Vec::new();
    for cal in parser {
        let cal = cal.into_diagnostic().wrap_err("failed to parse iCal")?;
        evs.extend(
            cal.events
                .into_iter()
                .flat_map(|ev| make_event(ev, offset).take_while(|x| x.start < limit)),
        );
    }

    evs.sort_by(|a, b| a.start.cmp(&b.start));

    Ok(evs)
}

fn make_event(ev: IcalEvent, offset: UtcOffset) -> impl Iterator<Item = Event> {
    let props = PropParser(&ev.properties);
    let mut rrule = props.rrule().map(|x| x.to_offset(offset));

    let first = (|| {
        let summary = props.str("SUMMARY")?;
        let start = props.datetime("DTSTART", offset)?.to_offset(offset);
        let end = props.datetime("DTEND", offset)?.to_offset(offset);
        Some(Event {
            summary,
            start,
            end,
        })
    })();

    std::iter::successors(first, move |ev| rrule.as_mut().and_then(|r| r.next(ev)))
}

struct PropParser<'a>(&'a [Property]);

impl<'a> PropParser<'a> {
    fn find(&self, name: &str) -> Option<&Property> {
        self.0.iter().find(|x| x.name == name)
    }

    fn parse<F, T>(&self, name: &str, f: F) -> Option<T>
    where
        F: FnOnce(&Property) -> Option<T>,
    {
        match self.find(name) {
            None => {
                log::warn!("could not find property name {name} in iCal");
                None
            }
            Some(p) => match f(p) {
                Some(x) => Some(x),
                None => {
                    log::warn!("failed to parse property value in {name} in iCal");
                    log::debug!("{p:?}");
                    None
                }
            },
        }
    }

    fn str(&self, name: &str) -> Option<String> {
        self.parse(name, |p| p.value.clone())
    }

    fn datetime(&self, name: &str, offset: UtcOffset) -> Option<OffsetDateTime> {
        self.parse(name, |p| {
            let val = p.value.as_ref()?;
            let tz = find_param(p, "TZID").or_else(|| find_param(p, "VALUE"));

            match tz.as_deref() {
                // only a date supplied, assume offset
                Some("DATE") => Date::parse(
                    val,
                    &iso8601::Iso8601::<
                        {
                            ICAL_DT
                                .set_formatted_components(iso8601::FormattedComponents::Date)
                                .encode()
                        },
                    >,
                )
                .ok()
                .map(|x| x.with_time(Time::MIDNIGHT).assume_offset(offset)),
                Some("Australia/Brisbane") => PrimitiveDateTime::parse(
                    val,
                    &iso8601::Iso8601::<
                        {
                            ICAL_DT
                                .set_formatted_components(iso8601::FormattedComponents::DateTime)
                                .encode()
                        },
                    >,
                )
                .ok()
                .map(|x| x.assume_offset(UtcOffset::from_hms(10, 0, 0).unwrap())),
                Some("Australia/Sydney") => {
                    let dt = PrimitiveDateTime::parse(
                        val,
                        &iso8601::Iso8601::<
                            {
                                ICAL_DT
                                    .set_formatted_components(
                                        iso8601::FormattedComponents::DateTime,
                                    )
                                    .encode()
                            },
                        >,
                    )
                    .ok()?;
                    let m = dt.month() as u8;
                    let h = if m <= 3 || m >= 10 { 11 } else { 10 };
                    Some(dt.assume_offset(UtcOffset::from_hms(h, 0, 0).unwrap()))
                }
                Some(id) => {
                    log::error!("unhandled TZID: {id}");
                    None
                }
                None => OffsetDateTime::parse(val, &iso8601::Iso8601::<{ ICAL_DT.encode() }>).ok(),
            }
        })
    }

    fn rrule(&self) -> Option<RepeatRule> {
        let p = self.find("RRULE")?;
        let x = p.value.as_deref().and_then(RepeatRule::parse);
        if x.is_none() {
            log::warn!("failed to parse property value in RRULE in iCal");
            log::debug!("{p:?}");
        }
        x
    }
}

#[derive(Default)]
struct RepeatRule {
    freq: Freq,
    until: Option<OffsetDateTime>,
    by_day: Option<(time::Weekday, i8)>,
    by_month_day: Option<u8>,
    interval: Option<u32>,
    count: Option<u32>,
}

impl RepeatRule {
    fn parse(s: &str) -> Option<Self> {
        let mut freq = None;
        let mut this = Self::default();

        for (key, val) in s.split(';').filter_map(|x| x.split_once('=')) {
            match key {
                "FREQ" => freq = Freq::parse(val),
                "UNTIL" => {
                    this.until = try_various_untils(val)
                        .expect("failed to parse UNTIL")
                        .into()
                }
                "BYDAY" => this.by_day = parse_by_day(val),
                "BYMONTHDAY" => this.by_month_day = val.parse::<u8>().expect("an integer").into(),
                "INTERVAL" => this.interval = val.parse::<u32>().expect("an integer").into(),
                "COUNT" => this.count = val.parse::<u32>().expect("an integer").into(),
                _ => (),
            }
        }

        this.freq = freq?;
        Some(this)
    }

    fn to_offset(self, offset: UtcOffset) -> Self {
        let Self {
            freq,
            until,
            by_day,
            by_month_day,
            interval,
            count,
        } = self;
        Self {
            freq,
            until: until.map(|x| x.to_offset(offset)),
            by_day,
            by_month_day,
            interval,
            count,
        }
    }

    fn filter_until(&self, start: OffsetDateTime) -> Option<OffsetDateTime> {
        match self.until {
            Some(u) if start < u => Some(start),
            None => Some(start),
            _ => None,
        }
    }

    fn next(&mut self, ev: &Event) -> Option<Event> {
        match &mut self.count {
            Some(0) => return None,
            Some(x) => *x -= 1,
            None => (),
        }

        let Self {
            freq,
            until: _,
            by_day,
            by_month_day,
            interval,
            count: _,
        } = self;
        match freq {
            Freq::Daily => {
                let start = std::iter::successors(Some(ev.start.date()), |x| x.next_day())
                    .nth(interval.unwrap_or(1) as usize)?;

                let start = ev.start.replace_date(start);
                let start = self.filter_until(start)?;
                let end = start + (ev.end - ev.start);

                Some(Event {
                    summary: ev.summary.clone(),
                    start,
                    end,
                })
            }
            Freq::Weekly => {
                let start = match by_day {
                    Some((d, _)) => Some(ev.start.date().next_occurrence(*d)),
                    None => std::iter::successors(Some(ev.start.date()), |x| x.next_day()).nth(7),
                }?;

                let start = ev.start.replace_date(start);
                let start = self.filter_until(start)?;
                let end = start + (ev.end - ev.start);

                Some(Event {
                    summary: ev.summary.clone(),
                    start,
                    end,
                })
            }
            Freq::Monthly => {
                let start =
                    std::iter::successors(Some(ev.start.date()), |x| x.next_day()).nth(32)?;

                let start = if let Some(d) = *by_month_day {
                    let d = d.min(time::util::days_in_year_month(start.year(), start.month()));
                    start.replace_day(d).unwrap()
                } else if let Some((day, i)) = *by_day {
                    if i > 0 {
                        start
                            .replace_day(1)
                            .unwrap()
                            .nth_next_occurrence(day, i as u8)
                    } else {
                        start
                            .replace_day(time::util::days_in_year_month(
                                start.year(),
                                start.month(),
                            ))
                            .unwrap()
                            .nth_prev_occurrence(day, (i * -1) as u8)
                    }
                } else {
                    start.replace_day(ev.start.day()).unwrap()
                };
                let start = ev.start.replace_date(start);
                let start = self.filter_until(start)?;
                let end = start + (ev.end - ev.start);

                Some(Event {
                    summary: ev.summary.clone(),
                    start,
                    end,
                })
            }
            Freq::Yearly => {
                let start = ev
                    .start
                    .replace_year(ev.start.year() + interval.unwrap_or(1) as i32)
                    .expect("should be fine");
                let start = self.filter_until(start)?;
                let end = start + (ev.end - ev.start);

                Some(Event {
                    summary: ev.summary.clone(),
                    start,
                    end,
                })
            }
        }
    }
}

fn try_various_untils(val: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(val, &iso8601::Iso8601::<{ ICAL_DT.encode() }>)
        .or_else(|_| {
            Date::parse(
                val,
                &iso8601::Iso8601::<
                    {
                        ICAL_DT
                            .set_formatted_components(iso8601::FormattedComponents::Date)
                            .encode()
                    },
                >,
            )
            .map(|x| x.with_time(Time::MIDNIGHT).assume_utc())
        })
        .ok()
}

fn parse_by_day(val: &str) -> Option<(Weekday, i8)> {
    fn parse_weekday(val: &str) -> Option<Weekday> {
        use time::Weekday::*;
        match val {
            "MO" => Some(Monday),
            "TU" => Some(Tuesday),
            "WE" => Some(Wednesday),
            "TH" => Some(Thursday),
            "FR" => Some(Friday),
            "SA" => Some(Saturday),
            "SU" => Some(Sunday),
            _ => None,
        }
    }

    fn parse_int(val: &str) -> (Option<i8>, &str) {
        let mut neg = false;
        let val = val
            .strip_prefix('-')
            .map(|v| {
                neg = true;
                v
            })
            .unwrap_or(val);

        val.chars()
            .next()
            .filter(|x| x.is_ascii_digit())
            .map(|x| (x.to_digit(10).map(|x| x as i8), &val[1..]))
            .unwrap_or((None, val))
    }

    let (i, val) = parse_int(val);
    parse_weekday(val).map(|w| (w, i.unwrap_or_default()))
}

#[derive(Default)]
enum Freq {
    Daily,
    #[default]
    Weekly,
    Monthly,
    Yearly,
}

impl Freq {
    fn parse(val: &str) -> Option<Self> {
        match val {
            "DAILY" => Some(Freq::Daily),
            "WEEKLY" => Some(Freq::Weekly),
            "MONTHLY" => Some(Freq::Monthly),
            "YEARLY" => Some(Freq::Yearly),
            _ => None,
        }
    }
}

fn find_param(prop: &Property, name: &str) -> Option<String> {
    prop.params
        .as_ref()?
        .iter()
        .find(|x| x.0 == name)
        .and_then(|x| x.1.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::Arbitrary;

    impl Arbitrary for Event {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                summary: String::arbitrary(g),
                start: crate::test::ArbitraryDateTime::arbitrary(g).0.assume_utc(),
                end: crate::test::ArbitraryDateTime::arbitrary(g).0.assume_utc(),
            }
        }
    }

    #[quickcheck]
    fn event_covers(ev: Event, date: crate::test::ArbitraryDateTime) -> bool {
        let date = date.0.date();
        let covers = std::iter::successors(Some(ev.start.date()), |x| x.next_day())
            .take_while(|&x| x <= ev.end.date())
            .find(|&x| x == date)
            .is_some();

        covers == ev.covers(date)
    }

    #[test]
    fn event_repetition() {
        use time::macros::datetime;
        let cal = "BEGIN:VCALENDAR
BEGIN:VEVENT
DTSTART;TZID=Australia/Brisbane:20240113T083000
DTEND;TZID=Australia/Brisbane:20240113T093000
RRULE:FREQ=WEEKLY;WKST=SU;UNTIL=20240119T135959Z;BYDAY=SA
SUMMARY:Test
END:VEVENT
BEGIN:VEVENT
DTSTART;TZID=Australia/Brisbane:20240120T083000
DTEND;TZID=Australia/Brisbane:20240120T093000
RRULE:FREQ=WEEKLY;WKST=SU;BYDAY=SA
SUMMARY:Test2
END:VEVENT
END:VCALENDAR";

        let cal = parse_ical(
            cal,
            UtcOffset::from_hms(10, 0, 0).unwrap(),
            datetime!(2024-02-10 0:00 +10),
        )
        .unwrap();

        assert_eq!(
            cal,
            vec![
                Event {
                    summary: "Test".to_string(),
                    start: datetime!(2024-01-13 8:30 +10),
                    end: datetime!(2024-01-13 9:30 +10),
                },
                Event {
                    summary: "Test2".to_string(),
                    start: datetime!(2024-01-20 8:30 +10),
                    end: datetime!(2024-01-20 9:30 +10),
                },
                Event {
                    summary: "Test2".to_string(),
                    start: datetime!(2024-01-27 8:30 +10),
                    end: datetime!(2024-01-27 9:30 +10),
                },
                Event {
                    summary: "Test2".to_string(),
                    start: datetime!(2024-02-03 8:30 +10),
                    end: datetime!(2024-02-03 9:30 +10),
                }
            ]
        );
    }
}
