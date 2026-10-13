use std::rc::Rc;

pub enum DialogState {
    ShowKeyValue {
        the_key: Rc<String>,
        value: Rc<String>,
    },
}
