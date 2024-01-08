use ical::{parser::ical::component::IcalEvent, property::Property};
use miette::*;
use time::{
    format_description::well_known::iso8601, Date, OffsetDateTime, PrimitiveDateTime, Time,
    UtcOffset,
};

const ICAL_DT: iso8601::Config = iso8601::Config::DEFAULT
    .set_use_separators(false)
    .set_time_precision(iso8601::TimePrecision::Second {
        decimal_digits: None,
    });

#[derive(Clone, Debug)]
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
pub fn parse_ical(data: &str, offset: UtcOffset) -> Result<Calendar> {
    let parser = ical::IcalParser::new(data.as_bytes());
    let mut evs = Vec::new();
    for cal in parser {
        let cal = cal.into_diagnostic().wrap_err("failed to parse iCal")?;
        evs.extend(
            cal.events
                .into_iter()
                .filter_map(|ev| make_event(ev, offset)),
        );
    }

    evs.sort_by(|a, b| a.start.cmp(&b.start));

    Ok(evs)
}

fn make_event(ev: IcalEvent, offset: UtcOffset) -> Option<Event> {
    let props = PropParser(&ev.properties);
    let summary = props.str("SUMMARY")?;
    let start = props.datetime("DTSTART")?.to_offset(offset);
    let end = props.datetime("DTEND")?.to_offset(offset);

    Event {
        summary,
        start,
        end,
    }
    .into()
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

    fn datetime(&self, name: &str) -> Option<OffsetDateTime> {
        self.parse(name, |p| {
            let val = p.value.as_ref()?;
            let tz = find_param(p, "TZID").or_else(|| find_param(p, "VALUE"));

            match tz.as_deref() {
                // only a date supplied, assume UTC (old format)
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
                .map(|x| x.with_time(Time::MIDNIGHT).assume_utc()),
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
}

fn find_param(prop: &Property, name: &str) -> Option<String> {
    prop.params
        .as_ref()?
        .iter()
        .find(|x| x.0 == name)
        .and_then(|x| x.1.get(0))
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
}
