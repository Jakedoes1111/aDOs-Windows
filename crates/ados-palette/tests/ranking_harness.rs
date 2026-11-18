//! Standalone pure-ranking check: rustc --edition=2021 --test this-file -o <output>.
//! Uses the registry action's data shape without linking the GUI dependency tree.
extern crate self as ados_core;
mod actions {
    #[derive(Clone)]
    pub struct PaletteAction {
        pub id: std::sync::Arc<str>,
        pub title: std::sync::Arc<str>,
        pub keywords: std::sync::Arc<str>,
        #[allow(dead_code)]
        pub shortcut: Option<std::sync::Arc<str>>,
        #[allow(dead_code)]
        pub run: std::rc::Rc<dyn Fn(&mut (), &mut ())>,
    }
}
#[allow(dead_code)]
#[path = "../src/ranking.rs"]
mod ranking;
