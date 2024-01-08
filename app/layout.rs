use std::collections::BTreeMap;

use crate::{
    data::{cal::Event, moon, weather, Model},
    render::Render,
};
use egui::{vec2, Align, Color32, Frame, Label, RichText, ScrollArea, Ui, Vec2};
use time::{macros::format_description, Date, OffsetDateTime, Weekday};

fn size_fonts(styles: &mut BTreeMap<egui::TextStyle, egui::FontId>, zoom: f32) {
    use egui::TextStyle::*;
    let f = egui::FontId::proportional;

    styles.insert(Small, f(9.0 * zoom));
    styles.insert(Body, f(12.5 * zoom));
    styles.insert(Monospace, f(12.0 * zoom));
    styles.insert(Button, f(12.5 * zoom));
    styles.insert(Heading, f(18.0 * zoom));
}

#[derive(Clone)]
pub struct Layout {
    pub zoom: f32,
    pub now: OffsetDateTime,
    pub mode: Mode,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            now: OffsetDateTime::now_utc(),
            mode: Mode::Month(Month),
        }
    }
}

impl Render<Model> for Layout {
    fn render(&self, ui: &mut Ui, model: Model) {
        let zoom = self.zoom;
        size_fonts(&mut ui.style_mut().text_styles, self.zoom);

        ui.columns(3, |cs| {
            let height = 20.0 * zoom;
            // left
            cs[0].set_height(height);
            cs[0].with_layout(egui::Layout::left_to_right(Align::BOTTOM), |ui| {
                let date = self
                    .now
                    .format(format_description!(
                        "[weekday] [day padding:none] [month repr:long] [year]"
                    ))
                    .unwrap_or_else(|_| "?".into());
                ui.heading(date);

                let ordinal = self
                    .now
                    .format(format_description!("[ordinal]"))
                    .unwrap_or_else(|_| "?".into());
                ui.small(format!("Day {ordinal}"));
            });

            // center
            cs[1].set_height(height);
            cs[1].with_layout(
                egui::Layout::right_to_left(Align::BOTTOM).with_main_justify(true),
                |ui| {
                    let time = self
                        .now
                        .format(format_description!("[hour repr:24]:[minute]"))
                        .unwrap_or_else(|_| "?".into());
                    ui.heading(time);
                },
            );

            // right
            cs[2].set_height(height);
            cs[2].with_layout(egui::Layout::right_to_left(Align::BOTTOM), |ui| {
                let fontsize = 20.0 * zoom;
                if let Some(weather) = model.weather.as_ref().map(|x| &x.current) {
                    if let Some(x) = weather.precipitation_prob {
                        ui.label(RichText::new(format!("({x:.0}%)")).size(fontsize));
                    }
                    weather_icon(ui, weather.code, fontsize);
                    if let Some(x) = weather.humidity {
                        ui.label(RichText::new(format!("💧{x:.0}%")).size(fontsize));
                    }
                    if let Some(t) = weather.temperature {
                        ui.label(RichText::new(format!("{t:.0}°C")).size(fontsize));
                    }
                }
                if let Some(moon) = model
                    .moon
                    .as_ref()
                    .and_then(|x| x.calendar.get(&self.now.date()))
                {
                    moon_icon(ui, moon.phase, fontsize);
                }
            });
        });

        self.mode.render(ui, (self, model));
    }
}

fn moon_icon(ui: &mut Ui, phase: moon::Phase, size: f32) {
    use moon::Phase::*;
    let txt = match phase {
        NewMoon => "🌑",
        WaxingCrescent => "🌘",
        FirstQuarter => "🌗",
        WaxingGibbous => "🌖",
        FullMoon => "🌕",
        WaningGibbous => "🌔",
        ThirdQuarter => "🌓",
        WaningCrescent => "🌒",
    };
    ui.label(RichText::new(txt).size(size));
}

fn weather_icon(ui: &mut Ui, code: weather::Code, size: f32) {
    use weather::Code::*;
    let txt = match code {
        ClearSky => "☀",
        MainlyClear => "🌤",
        PartlyCloudy => "⛅",
        Overcast => "☁",
        Fog => "🌫",
        Drizzle | Rain => "☔",
        Snow => "🌨",
        Thuderstorm => "⚡",
    };
    ui.label(RichText::new(txt).size(size));
}

// ##### MODE ##################################################################

#[derive(Clone)]
pub enum Mode {
    Month(Month),
}

impl Render<(&Layout, Model)> for Mode {
    fn render(&self, ui: &mut Ui, ctx: (&Layout, Model)) {
        match self {
            Mode::Month(month) => month.render(ui, ctx),
        }
    }
}

// ##### MONTH #################################################################

#[derive(Default, Copy, Clone)]
pub struct Month;

impl Render<(&Layout, Model)> for Month {
    fn render(&self, ui: &mut Ui, (layout, model): (&Layout, Model)) {
        let mut evs = model.cals.values().flatten().collect::<Vec<_>>();
        evs.sort_by(|a, b| a.start.cmp(&b.start));

        let zoom = layout.zoom;
        ui.spacing_mut().item_spacing = Vec2::ZERO;

        // day headers
        ui.columns(7, |cs| {
            std::iter::successors(Some(Weekday::Monday), |x| x.next().into())
                .zip(cs)
                .for_each(|(d, ui)| {
                    Frame::none()
                        .stroke((1. * zoom, Color32::BLACK))
                        .inner_margin(2.0 * zoom)
                        .show(ui, |ui| {
                            ui.vertical_centered_justified(|ui| ui.label(d.to_string()))
                        });
                });
        });

        let start = week_start(layout.now.date().replace_day(1).unwrap());
        let end = week_end(end_of_month(layout.now.date()));
        let days = std::iter::successors(Some(start), |x| x.next_day())
            .take_while(|x| x <= &end)
            .collect::<Vec<_>>();
        debug_assert_eq!(days.len() % 7, 0);

        let weeks = days.chunks_exact(7);
        let week_height = ui.available_height() / weeks.len() as f32;
        let mut evs = evs.as_slice();
        for week in weeks {
            ui.columns(7, |cs| {
                week.iter().copied().zip(cs).for_each(|(day, ui)| {
                    // progressively shrink the slice
                    evs = remove_earlier_events(evs, day);
                    let cell = CellWidget {
                        zoom,
                        is_today: day == layout.now.date(),
                        day,
                        model: &model,
                    };
                    ui.allocate_ui(vec2(ui.available_width(), week_height), |ui| {
                        cell.day_cell(ui, evs);
                    });
                });
            });
        }
    }
}

fn end_of_month(date: Date) -> Date {
    date.replace_day(time::util::days_in_year_month(date.year(), date.month()))
        .unwrap()
}

fn week_start(date: Date) -> Date {
    if date.weekday() == Weekday::Monday {
        date
    } else {
        date.prev_occurrence(Weekday::Monday)
    }
}

fn week_end(date: Date) -> Date {
    if date.weekday() == Weekday::Sunday {
        date
    } else {
        date.next_occurrence(Weekday::Sunday)
    }
}

fn remove_earlier_events<'a>(evs: &'a [&'a Event], before: Date) -> &'a [&'a Event] {
    let i = evs
        .iter()
        .enumerate()
        .find(|(_, e)| e.start.date() >= before || e.covers(before))
        .map(|(i, _)| i)
        .unwrap_or(evs.len());
    &evs[i..]
}

struct CellWidget<'a> {
    zoom: f32,
    is_today: bool,
    day: Date,
    model: &'a Model,
}

impl<'a> CellWidget<'a> {
    fn day_cell(&self, ui: &mut Ui, evs: &[&Event]) {
        let Self {
            zoom,
            is_today: _,
            day,
            model: _,
        } = *self;
        Frame::none()
            .stroke((1. * zoom, Color32::BLACK))
            .inner_margin(2.0 * zoom)
            .show(ui, |ui| {
                self.day_header(ui);

                // events
                ScrollArea::new([false, true])
                    .id_source(day.to_string())
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        evs.iter()
                            .take_while(|x| x.start.date() <= day)
                            .filter(|x| x.covers(day))
                            .for_each(|e| self.event_line(ui, e));
                    });

                // pad out
                ui.allocate_space(ui.available_size());
            });
    }

    fn day_header(&self, ui: &mut Ui) {
        let Self {
            zoom,
            is_today,
            day,
            model,
        } = *self;
        let (frame, dark) = if is_today {
            (Frame::none().fill(Color32::DARK_GRAY), true)
        } else {
            (Frame::none(), false)
        };
        frame.show(ui, |ui| {
            if dark {
                ui.visuals_mut().override_text_color = Some(Color32::WHITE);
            }
            ui.set_height(16.0 * zoom);
            ui.horizontal_centered(|ui| {
                ui.label(day.day().to_string());

                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if let Some(weather) = model.weather.as_ref().and_then(|x| x.forecast.get(&day))
                    {
                        if let Some(x) = weather.precipitation_prob {
                            ui.label(RichText::new(format!("({x:.0}%)")).size(10.0 * zoom));
                        }
                        weather_icon(ui, weather.code, 14.0 * zoom);
                        if let Some(t) = weather.temperature {
                            ui.label(format!("{t:.0}°C"));
                        }
                    }
                    if let Some(moon) = model.moon.as_ref().and_then(|x| x.calendar.get(&day)) {
                        moon_icon(ui, moon.phase, 14.0 * zoom);
                    }
                });
            });
        });
    }

    fn event_line(&self, ui: &mut Ui, event: &Event) {
        let Self {
            zoom,
            is_today: _,
            day,
            model: _,
        } = *self;
        let Event {
            summary,
            start,
            end: _,
        } = event;

        ui.horizontal(|ui| {
            ui.set_height(10.0 * zoom);
            ui.spacing_mut().item_spacing.x = 2.0 * zoom;
            let rt = if start.date() == day {
                RichText::new(format!("{:02}:{:02}", start.hour(), start.minute()))
            } else {
                RichText::new("⬅")
            };
            ui.label(rt.strong().small());
            ui.add(Label::new(RichText::new(summary).small()).truncate(true));
        });
    }
}
