use event::PluginEventKind;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

pub mod event;

pub trait LapcePlugin {
    fn initialize(&mut self, configuration: serde_json::Value) {}

    fn update(&mut self, event: serde_json::Value) {}
}

#[macro_export]
macro_rules! register_plugin {
    ($t:ty) => {
        thread_local! {
            static STATE: std::cell::RefCell<$t> = std::cell::RefCell::new(Default::default());
        }

        fn main() {}

        #[no_mangle]
        fn initialize() {
            STATE.with(|state| {
                state
                    .borrow_mut()
                    .initialize($crate::object_from_stdin().unwrap());
            });
        }

        #[no_mangle]
        fn update() {
            STATE.with(|state| {
                state
                    .borrow_mut()
                    .update($crate::object_from_stdin().unwrap());
            });
        }
    };
}

pub fn object_from_stdin<T: DeserializeOwned>() -> Result<T, serde_json::Error> {
    let mut json = String::new();
    std::io::stdin().read_line(&mut json).unwrap();
    serde_json::from_str(&json)
}

pub fn object_to_stdout(object: &impl Serialize) {
    println!("{}", serde_json::to_string(object).unwrap());
}

pub fn send_notification(method: &str, params: &Value) {
    object_to_stdout(&serde_json::json!({
        "method": method,
        "params": params,
    }));
    unsafe { host_handle_notification() };
}

/// Subscribe to specific events to receive them in the [`LapcePlugin::update`] method
pub fn subscribe(events: &[PluginEventKind]) {
    send_notification(
        "subscribe",
        &json!({
            "events": events,
        }),
    )
}

pub fn start_lsp(exec_path: &str, language_id: &str, options: Option<Value>) {
    send_notification(
        "start_lsp_server",
        &json!({
            "exec_path": exec_path,
            "language_id": language_id,
            "options": options,
        }),
    );
}

#[link(wasm_import_module = "lapce")]
extern "C" {
    fn host_handle_notification();
}
