use std::{collections::HashMap, ops::Deref, sync::Arc};

pub mod cal;
pub mod moon;
pub mod weather;

#[derive(Clone, Default)]
pub struct Model(Arc<Model_>);

#[derive(Default, Clone)]
pub struct Model_ {
    pub cals: HashMap<String, cal::Calendar>,
    pub weather: Option<weather::Weather>,
    pub moon: Option<moon::LunarCalendar>,
}

impl Deref for Model {
    type Target = Model_;
    fn deref(&self) -> &Model_ {
        self.0.as_ref()
    }
}

impl Model {
    pub fn make_mut(&mut self) -> &mut Model_ {
        Arc::make_mut(&mut self.0)
    }
}
