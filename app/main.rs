use miette::*;
use pical::state::Dispatch;
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    time::{Duration, Instant},
};
use time::{OffsetDateTime, UtcOffset};
use tokio::{
    io::AsyncWriteExt,
    sync::Mutex,
    time::{interval, MissedTickBehavior},
};

fn main() -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .into_diagnostic()?
        .block_on(main_())
}

async fn main_() -> Result<()> {
    init_logging()?;

    let cpath = "./config.pical.toml";
    let Config {
        width,
        height,
        zoom,
        scaling,
        display_refresh,
        timezone,
        calendars,
        coords,
        stormglassio_apikey,
    } = Config::read_or_default(cpath).await?;
    log::info!("✅ read in config from {cpath}");

    #[cfg(not(feature = "local"))]
    start_it8951_driver().await?;
    let state = State {
        layout: pical::layout::Layout {
            zoom,
            mode: pical::layout::TwelveDay.into(),
            ..Default::default()
        },
        push_bitmap: |img, old| Box::pin(async move { push_bitmap(&img, old.as_deref()).await }),
        ..Default::default()
    };

    let (dispatch, state_loop) = pical::state::dispatcher(state);
    tokio::spawn(state_loop);

    tokio::spawn(clock_loop(
        dispatch.clone(),
        Duration::from_secs(31),
        timezone,
    ));
    tokio::spawn(fetch_loop(
        dispatch.clone(),
        coords,
        calendars,
        stormglassio_apikey,
        Duration::from_secs(61),
    )?);
    render_loop(dispatch, display_refresh, width, height, scaling).await
}

fn init_logging() -> Result<()> {
    let lvl = log::LevelFilter::Debug;
    let config = simplelog::ConfigBuilder::default()
        .add_filter_allow_str("pical")
        .build();
    simplelog::CombinedLogger::init(vec![
        simplelog::WriteLogger::new(
            lvl,
            config.clone(),
            std::fs::File::create("pical.log").into_diagnostic()?,
        ),
        simplelog::TermLogger::new(
            lvl,
            config,
            Default::default(),
            simplelog::ColorChoice::Auto,
        ),
    ])
    .into_diagnostic()
    .wrap_err("initialising logging failed")
}

#[derive(Serialize, Deserialize)]
struct Config {
    width: u32,
    height: u32,
    zoom: f32,
    scaling: f32,
    #[serde(with = "humantime_serde")]
    display_refresh: Duration,
    timezone: UtcOffset,
    calendars: Vec<(String, String)>,
    coords: [f32; 2],
    stormglassio_apikey: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            zoom: 1.0,
            scaling: 1.0,
            display_refresh: Duration::from_secs(30),
            timezone: UtcOffset::UTC,
            calendars: vec![(
                "Name".to_string(),
                "https://calendar.google.com/calendar/ical/path-to-cal".to_string(),
            )],
            coords: [0.; 2],
            stormglassio_apikey: String::new(),
        }
    }
}

impl Config {
    async fn read_or_default(path: &str) -> Result<Self> {
        let path = Path::new(path);
        if path.exists() {
            let s = tokio::fs::read_to_string(path)
                .await
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to read {}", path.display()))?;
            toml::from_str(&s).into_diagnostic().wrap_err_with(|| {
                format!("failed to deserialize config in {} to TOML", path.display())
            })
        } else {
            let cfg = Self::default();
            let toml = toml::to_string_pretty(&cfg).expect("should serialize just fine");
            tokio::fs::write(path, toml)
                .await
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to write config to {}", path.display()))?;
            Ok(cfg)
        }
    }
}

struct State {
    model: pical::data::Model,
    layout: pical::layout::Layout,
    push_bitmap: fn(PathBuf, Option<PathBuf>) -> Pin<Box<dyn Future<Output = Result<()>>>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            model: Default::default(),
            layout: Default::default(),
            push_bitmap: |_path, _old| {
                Box::pin(async { Err(miette!("provide a push_bitmap function")) })
            },
        }
    }
}

fn log_error(e: Report) {
    let mut buf = String::new();
    let _ = GraphicalReportHandler::new().render_report(&mut buf, e.as_ref());
    log::error!("{}", buf);
}

async fn render_loop(
    dispatch: Dispatch<State>,
    refresh: Duration,
    width: u32,
    height: u32,
    scaling: f32,
) -> Result<()> {
    use pical::render::Render;

    let mut timer = interval(refresh);
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        timer.tick().await;
        let (data, layout, push_bitmap) = dispatch
            .run(|s| (s.model.clone(), s.layout.clone(), s.push_bitmap))
            .await;

        let now = std::time::Instant::now();
        let img = pical::render::paint(width, height, scaling, |ctx| {
            ctx.set_visuals(egui::Visuals::light());
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(egui::Color32::WHITE))
                .show(ctx, |ui| layout.render(ui, data));
        });
        let render_time = now.elapsed();
        img.log_debug_timings();
        let img = img.img;

        let now = std::time::Instant::now();
        let path = "./frame.pical.bmp";
        let old = match save_img(img, path) {
            Ok(x) => x,
            Err(e) => {
                log_error(e);
                continue;
            }
        };
        let save_time = now.elapsed();

        let now = std::time::Instant::now();
        if let Err(e) = push_bitmap(path.into(), old)
            .await
            .wrap_err_with(|| format!("failed to push bitmap to {path}"))
        {
            log_error(e);
            continue;
        }
        let push_time = now.elapsed();

        log::info!(
            "⏱ Render perf: rendering=>{} | save-bitmap=>{} | push-time=>{}",
            humantime::Duration::from(render_time),
            humantime::Duration::from(save_time),
            humantime::Duration::from(push_time)
        );
    }
}

/// Returns if an original file at `to` was renamed.
fn save_img(img: impl Into<image::DynamicImage>, to: &str) -> Result<Option<PathBuf>> {
    let to = Path::new(to);
    let old = if to.exists() {
        let mut o = format!(
            "{}.old",
            to.file_stem().and_then(|x| x.to_str()).unwrap_or_default()
        );
        if let Some(ext) = to.extension().and_then(|x| x.to_str()) {
            o.push('.');
            o.push_str(ext);
        }
        let o = to.with_file_name(o);
        std::fs::rename(to, &o).into_diagnostic()?;
        Some(o)
    } else {
        None
    };

    let img = img.into().into_luma8();
    img.save(to)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to save bitmap to {}", to.display()))?;
    Ok(old)
}

async fn clock_loop(dispatch: Dispatch<State>, every: Duration, offset: UtcOffset) {
    let mut timer = interval(every);
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        dispatch
            .run(move |s| {
                s.layout.now = OffsetDateTime::now_utc().to_offset(offset);
            })
            .await;
        timer.tick().await;
    }
}

fn fetch_loop(
    dispatch: Dispatch<State>,
    coords: [f32; 2],
    cals: Vec<(String, String)>,
    stormglassio_apikey: String,
    every: Duration,
) -> Result<impl Future<Output = ()>> {
    let mut timer = interval(every);
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .into_diagnostic()
        .wrap_err("failed to build reqwest client")?;

    Ok(async move {
        loop {
            if let Err(e) =
                fetch_iteration(&dispatch, &client, &cals, coords, &stormglassio_apikey).await
            {
                log_error(e);
            }
            timer.tick().await;
        }
    })
}

async fn fetch_iteration(
    dispatch: &Dispatch<State>,
    client: &reqwest::Client,
    calendars: &[(String, String)],
    coords: [f32; 2],
    stormglassio_apikey: &str,
) -> Result<()> {
    let (model, now) = dispatch
        .run(|state| (state.model.clone(), state.layout.now))
        .await;

    // download the calendar(s)
    let mut cals = Vec::with_capacity(calendars.len());
    let limit = std::iter::successors(Some(now.date()), |x| x.next_day())
        .nth(60)
        .map(|d| now.replace_date(d))
        .unwrap_or(now);
    for (name, url) in calendars {
        let ical = pical::fetch::string(client, url, [])
            .await
            .and_then(|x| pical::data::cal::parse_ical(&x, now.offset(), limit))?;
        cals.push((name.clone(), ical));
        log::info!("Fetched latest calendars");
    }

    // fetch the weather
    // only do this every 10 minutes to avoid making execessive API calls
    let mut weather = None;
    if model
        .weather
        .as_ref()
        .map(|x| Instant::now().duration_since(x.last_update) > Duration::from_secs(60 * 10))
        .unwrap_or(true)
    {
        let [lat, long] = coords;
        let tz = now.offset();
        let url = reqwest::Url::parse_with_params(
            "https://api.open-meteo.com/v1/forecast?\
                current=temperature_2m,relative_humidity_2m,precipitation,weather_code&\
                daily=weather_code,temperature_2m_max,precipitation_probability_max&\
                forecast_days=16",
            &[
                ("latitude", lat.to_string()),
                ("longitude", long.to_string()),
                ("timezone", format!("GMT{:+}", tz.whole_hours())),
            ],
        )
        .into_diagnostic()
        .wrap_err("URL parse failed")?;
        let url = url.as_str();
        let resp = pical::fetch::json(client, url, []).await?;
        weather = Some(pical::data::weather::Weather::from_open_meteo(resp)?);
        log::info!("Fetched latest weather");
    }

    // fetch lunar calendar
    // only do this every half a day -- avoids rate limits and will not change
    let mut moon = None;
    if model
        .moon
        .as_ref()
        .map(|x| Instant::now().duration_since(x.last_update) > Duration::from_secs(60 * 60 * 12))
        .unwrap_or(true)
    {
        let [lat, long] = coords;
        let url = reqwest::Url::parse_with_params(
            "https://api.stormglass.io/v2/astronomy/point",
            &[
                ("lat", lat.to_string()),
                ("lng", long.to_string()),
                ("start", now.date().to_string()),
                ("end", (now.date() + time::Duration::days(10)).to_string()),
            ],
        )
        .into_diagnostic()
        .wrap_err("URL parse failed")?;
        let url = url.as_str();
        let resp = pical::fetch::json(
            client,
            url,
            [("Authorization", stormglassio_apikey.to_string())],
        )
        .await?;
        moon = Some(pical::data::moon::LunarCalendar::from_storm_glass_io(
            resp,
            now.offset(),
        )?);
        log::info!("Fetched latest lunar calendar");
    }

    drop(model); // drop ref count
    dispatch
        .run(|state| {
            let model = state.model.make_mut();
            for (key, cal) in cals {
                model.cals.insert(key.to_string(), cal);
            }
            if let Some(w) = weather {
                model.weather = Some(w);
            }
            if let Some(m) = moon {
                model.moon = Some(m);
            }
        })
        .await;

    Ok(())
}

static DRIVER_PROCESS: Mutex<Option<ScreenDriver>> = Mutex::const_new(None);

struct ScreenDriver {
    process: tokio::process::Child,
    count: u8,
    reset_count: u16,
}

async fn start_it8951_driver() -> Result<()> {
    *DRIVER_PROCESS.lock().await = Some(ScreenDriver::start()?);
    Ok(())
}

impl ScreenDriver {
    fn start() -> Result<Self> {
        use tokio::process::*;
        let child = Command::new("./it8951-driver")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .into_diagnostic()
            .wrap_err("failed to start ./it8951-driver")?;
        Ok(ScreenDriver {
            process: child,
            count: 0,
            reset_count: 0,
        })
    }
}

/// Change this to suit the how to push a frame to the screen.
async fn push_bitmap(img: &Path, old: Option<&Path>) -> Result<()> {
    let mut child_ = DRIVER_PROCESS.lock().await;
    let child = child_
        .as_mut()
        .ok_or_else(|| miette!("it8951-driver process not started"))?;
    child.count += 1;
    child.reset_count += 1;
    let mut line = img.display().to_string();
    if child.count > 10 {
        // do high screen
        child.count = 0;
        line += " --high";
    } else {
        // add maybe diff
        if let Some(diff) = old {
            line += " --low ";
            line += &diff.display().to_string();
        }
    }

    line.push('\n'); // new line to end

    let x = tokio::time::timeout(Duration::from_secs(60), async {
        match &mut child.process.stdin {
            Some(child) => child.write_all(line.as_bytes()).await.into_diagnostic(),
            None => Err(miette!("no stdin pipe for it8951-driver")),
        }
    })
    .await;

    let reset = match x {
        Ok(res) => {
            res?;
            false
        }
        // timed out
        Err(e) => {
            log::error!("{e}");
            true
        }
    };

    if child.reset_count > 180 || reset {
        log::warn!("Restarting it8951-driver processing");
        child.process.kill().await.into_diagnostic()?;
        *child = ScreenDriver::start()?;
    }

    Ok(())
}
