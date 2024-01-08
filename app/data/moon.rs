use miette::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Instant;
use time::{Date, OffsetDateTime, UtcOffset};

#[derive(Clone)]
pub struct LunarCalendar {
    pub last_update: Instant,
    pub calendar: HashMap<Date, Moon>,
}

#[derive(Clone)]
pub struct Moon {
    pub phase: Phase,
}

#[derive(Clone, Copy)]
pub enum Phase {
    NewMoon,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    FullMoon,
    WaningGibbous,
    ThirdQuarter,
    WaningCrescent,
}

impl LunarCalendar {
    pub fn from_storm_glass_io(payload: StormGlassPayload, offset: UtcOffset) -> Result<Self> {
        let mut calendar = HashMap::default();
        let fmt = time::format_description::well_known::Iso8601::PARSING;
        for StormGlassData { time, moon_phase } in payload.data {
            let date = OffsetDateTime::parse(&time, &fmt)
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to parse time {time}"))?
                .to_offset(offset)
                .date();
            let phase = Phase::from_storm_glass_io(&moon_phase.current.text)?;

            calendar.insert(date, Moon { phase });
        }

        Ok(Self {
            last_update: Instant::now(),
            calendar,
        })
    }
}

impl Phase {
    fn from_storm_glass_io(text: &str) -> Result<Self> {
        use Phase::*;
        match text {
            "New moon" => Ok(NewMoon),
            "Waxing crescent" => Ok(WaxingCrescent),
            "First quarter" => Ok(FirstQuarter),
            "Waxing gibbous" => Ok(WaxingGibbous),
            "Full moon" => Ok(FullMoon),
            "Waning gibbous" => Ok(WaningGibbous),
            "Third quarter" => Ok(ThirdQuarter),
            "Waning crescent" => Ok(WaningCrescent),
            x => Err(miette!("unknown moon phase: {x}")),
        }
    }
}

#[derive(Deserialize)]
pub struct StormGlassPayload {
    data: Vec<StormGlassData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StormGlassData {
    time: String,
    moon_phase: StormGlassMoonPhase,
}

#[derive(Deserialize)]
struct StormGlassMoonPhase {
    current: StormGlassMoonPhaseObj,
}

#[derive(Deserialize)]
struct StormGlassMoonPhaseObj {
    text: String,
}
