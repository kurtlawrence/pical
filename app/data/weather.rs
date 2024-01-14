use miette::*;
use serde::Deserialize;
use std::{collections::HashMap, time::Instant};
use time::Date;

#[derive(Clone)]
pub struct Weather {
    pub last_update: Instant,
    pub current: Ob,
    pub forecast: HashMap<Date, Ob>,
}

#[derive(Clone)]
pub struct Ob {
    pub code: Code,
    pub temperature: Option<f32>,
    pub humidity: Option<f32>,
    pub precipitation_prob: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
pub enum Code {
    ClearSky,
    MainlyClear,
    PartlyCloudy,
    Overcast,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thuderstorm,
}

impl Weather {
    pub fn from_open_meteo(payload: OpenMeteoPayload) -> Result<Self> {
        let OpenMeteoPayload { current, daily } = payload;

        let OpenMeteoCurrent {
            temperature_2m,
            relative_humidity_2m,
            weather_code,
        } = current;
        let current = Ob {
            code: Code::from_open_meteo(weather_code)?,
            temperature: temperature_2m.into(),
            humidity: Some(relative_humidity_2m),
            precipitation_prob: None,
        };

        let OpenMeteoDaily {
            time,
            weather_code,
            temperature_2m_max,
            precipitation_probability_max,
        } = daily;
        let mut forecast = HashMap::default();
        for (((date, code), temperature), precipitation_prob) in time
            .into_iter()
            .zip(weather_code)
            .zip(temperature_2m_max)
            .zip(precipitation_probability_max)
        {
            let date = Date::parse(&date, &time::format_description::well_known::Iso8601::DATE)
                .into_diagnostic()
                .wrap_err_with(|| format!("date value: {date}"))?;
            let code = code
                .ok_or_else(|| miette!("no weather code for {date}"))
                .and_then(Code::from_open_meteo)?;
            let ob = Ob {
                code,
                temperature,
                precipitation_prob,
                humidity: None,
            };

            forecast.insert(date, ob);
        }

        Ok(Self {
            last_update: Instant::now(),
            current,
            forecast,
        })
    }
}

impl Code {
    fn from_open_meteo(code: u32) -> Result<Self> {
        use Code::*;
        match code {
            0 => Ok(ClearSky),
            1 => Ok(MainlyClear),
            2 => Ok(PartlyCloudy),
            3 => Ok(Overcast),
            45 | 48 => Ok(Fog),
            51 | 53 | 55 | 56 | 57 => Ok(Drizzle),
            61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => Ok(Rain),
            71 | 73 | 75 | 77 | 85 | 86 => Ok(Snow),
            95 | 96 | 99 => Ok(Thuderstorm),
            x => Err(miette!("weather code {} is not handled", x)),
        }
    }
}

#[derive(Deserialize)]
pub struct OpenMeteoPayload {
    current: OpenMeteoCurrent,
    daily: OpenMeteoDaily,
}

#[derive(Deserialize)]
struct OpenMeteoCurrent {
    temperature_2m: f32,
    relative_humidity_2m: f32,
    weather_code: u32,
}

#[derive(Deserialize)]
struct OpenMeteoDaily {
    time: Vec<String>,
    weather_code: Vec<Option<u32>>,
    temperature_2m_max: Vec<Option<f32>>,
    precipitation_probability_max: Vec<Option<f32>>,
}
