use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum RightPanelState {
    ShowServiceOverview,
    ShowServiceDataOverview(Rc<String>),
    ShowProcess(Rc<String>, i64),
}
