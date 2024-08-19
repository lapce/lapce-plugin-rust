pub mod lsp;

use std::{
    env,
    num::ParseIntError,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use jsonrpc_lite::{Id, JsonRpc};
use lapce_wasi_experimental_http::Response;
use lsp::LspRef;
use once_cell::sync::Lazy;
pub use psp_types;
use psp_types::{
    lsp_types::{
        notification::{LogMessage, ShowMessage},
        DocumentSelector, LogMessageParams, MessageType, ShowMessageParams, Uri,
    },
    ExecuteProcess, ExecuteProcessParams, ExecuteProcessResult, LspId, Notification,
    RegisterDebuggerType, RegisterDebuggerTypeParams, Request, SendLspNotification,
    SendLspNotificationParams, SendLspRequest, SendLspRequestParams, SendLspRequestResult,
    StartLspServer, StartLspServerParams, StartLspServerResult,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use thiserror::Error;

pub static PLUGIN_RPC: Lazy<PluginServerRpcHandler> = Lazy::new(PluginServerRpcHandler::new);

#[derive(Error, Debug)]
pub enum PluginError {
    #[error("serde related errors:{0}")]
    Serde(#[from] serde_json::Error),
    #[error("HTTP related errors:{0}")]
    Http(#[from] http::Error),
    #[error("JSON-RPC related errors:{0}")]
    JsonRpc(#[from] jsonrpc_lite::Error),
    #[error("I/O related errors:{0}")]
    Io(#[from] std::io::Error),
    #[error("Anyhow errors:{0}")]
    Anyhow(#[from] anyhow::Error),
    #[error("Unable to parse string as number:{0}")]
    ParseInt(#[from] ParseIntError),
    #[error("Other errors:{0}")]
    Other(String),
}

/// Helper struct abstracting environment variables
/// names used in lapce to provide revelant host
/// environment information, so plugin maintainers
/// don't have to hardcode specific variable names
pub struct VoltEnvironment {}

impl VoltEnvironment {
    /// Plugin location path encoded as Url
    pub fn uri() -> Result<String, env::VarError> {
        env::var("VOLT_URI")
    }

    /// Operating system name as provided by
    /// std::env::consts::OS
    pub fn operating_system() -> Result<String, env::VarError> {
        env::var("VOLT_OS")
    }

    /// Processor architecture name as provided by
    /// std::env::consts::ARCH
    pub fn architecture() -> Result<String, env::VarError> {
        env::var("VOLT_ARCH")
    }

    /// C library used on host detected by parsing ldd output
    /// provided because of musl-based linux distros and distros
    /// that need statically linked binaries due to how
    /// linking works (e.g. nixOS)
    /// Currently only 2 options are available: glibc | musl
    /// This function will return empty string on non-linux
    /// hosts
    pub fn libc() -> Result<String, env::VarError> {
        env::var("VOLT_LIBC")
    }
}

#[allow(unused_variables)]
pub trait LapcePlugin {
    fn handle_request(&mut self, id: u64, method: String, params: Value) {}
    fn handle_notification(&mut self, method: String, params: Value) {}
}

pub enum PluginServerRpc {
    Request {
        id: u64,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

pub struct PluginServerRpcHandler {
    id: Arc<AtomicU64>,
}

pub struct Http {}

impl Http {
    pub fn get(url: &str) -> Result<Response, PluginError> {
        let req = http::request::Builder::new()
            .method(http::Method::GET)
            .uri(url)
            .body(None)?;
        let resp = lapce_wasi_experimental_http::request(req)?;
        Ok(resp)
    }
}

#[macro_export]
macro_rules! register_plugin {
    ($t:ty) => {
        thread_local! {
            static STATE: std::cell::RefCell<$t> = std::cell::RefCell::new(Default::default());
        }

        fn main() {}

        #[no_mangle]
        pub fn handle_rpc() {
            if let Ok(rpc) = $crate::parse_stdin() {
                match rpc {
                    $crate::PluginServerRpc::Request { id, method, params } => {
                        STATE.with(|state| {
                            state.borrow_mut().handle_request(id, method, params);
                        });
                    }
                    $crate::PluginServerRpc::Notification { method, params } => {
                        STATE.with(|state| {
                            state.borrow_mut().handle_notification(method, params);
                        });
                    }
                }
            }
        }
    };
}

impl PluginServerRpcHandler {
    fn new() -> Self {
        Self {
            id: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn stderr(&self, msg: &str) {
        eprintln!("{msg}");
        unsafe { host_handle_stderr() };
    }

    pub fn window_log_message(
        &self,
        kind: MessageType,
        message: String,
    ) -> Result<(), PluginError> {
        self.host_notification(LogMessage::METHOD, LogMessageParams { typ: kind, message })
    }

    pub fn window_show_message(
        &self,
        kind: MessageType,
        message: String,
    ) -> Result<(), PluginError> {
        self.host_notification(
            ShowMessage::METHOD,
            ShowMessageParams { typ: kind, message },
        )
    }

    pub fn start_lsp(
        &self,
        server_uri: Uri,
        server_args: Vec<String>,
        document_selector: DocumentSelector,
        options: Option<Value>,
    ) -> Result<LspRef, PluginError> {
        let res: StartLspServerResult = self.host_request(
            StartLspServer::METHOD,
            StartLspServerParams {
                server_uri,
                server_args,
                document_selector,
                options,
            },
        )?;

        let id = res.id;
        Ok(LspRef::new(id))
    }

    pub fn register_debugger_type(
        &self,
        debugger_type: String,
        program: String,
        args: Option<Vec<String>>,
    ) -> Result<(), PluginError> {
        self.host_request(
            RegisterDebuggerType::METHOD,
            RegisterDebuggerTypeParams {
                debugger_type,
                program,
                args,
            },
        )
    }

    /// Send a notification to an LSP.  
    pub fn lsp_send_notification<P: Serialize>(
        &self,
        lsp_id: LspId,
        method: &str,
        params: P,
    ) -> Result<(), PluginError> {
        self.host_notification(
            SendLspNotification::METHOD,
            SendLspNotificationParams {
                id: lsp_id,
                method: method.to_string(),
                params: serde_json::to_value(params)?,
            },
        )
    }

    /// Send a request to an LSP.  
    /// This *waits* for the client to respond with a result.
    pub fn lsp_send_request_blocking<P: Serialize, D: DeserializeOwned>(
        &self,
        lsp_id: LspId,
        method: &str,
        params: P,
    ) -> Result<D, PluginError> {
        let res: SendLspRequestResult = self.host_request(
            SendLspRequest::METHOD,
            SendLspRequestParams {
                id: lsp_id,
                method: method.to_string(),
                params: serde_json::to_value(params)?,
            },
        )?;

        Ok(serde_json::from_value(res.result)?)
    }

    pub fn execute_process(
        &self,
        program: String,
        args: Vec<String>,
    ) -> Result<ExecuteProcessResult, PluginError> {
        self.host_request(
            ExecuteProcess::METHOD,
            ExecuteProcessParams { program, args },
        )
    }

    fn host_request<P: Serialize, D: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<D, PluginError> {
        let id = self.id.fetch_add(1, Ordering::Relaxed);
        let params = serde_json::to_value(params)?;
        send_host_request(id, method, &params)?;
        let mut msg = String::new();
        std::io::stdin().read_line(&mut msg)?;

        match JsonRpc::parse(&msg) {
            Ok(rpc) => {
                if let Some(value) = rpc.get_result() {
                    let result = serde_json::from_value::<D>(value.clone())?;
                    Ok(result)
                } else if let Some(err) = rpc.get_error() {
                    Err(PluginError::JsonRpc(err.clone()))
                } else {
                    Err(PluginError::JsonRpc(jsonrpc_lite::Error::invalid_request()))
                }
            }
            _ => Err(PluginError::JsonRpc(jsonrpc_lite::Error::invalid_request())),
        }
    }

    fn host_notification<P: Serialize>(&self, method: &str, params: P) -> Result<(), PluginError> {
        let params = serde_json::to_value(params)?;
        send_host_notification(method, &params)?;
        Ok(())
    }

    pub fn host_success<P: Serialize>(&self, id: u64, params: P) -> Result<(), PluginError> {
        let params = serde_json::to_value(params)?;
        send_host_success(id, &params)?;
        Ok(())
    }

    pub fn host_error<P: AsRef<str>>(&self, id: u64, params: P) -> Result<(), PluginError> {
        send_host_error(id, params.as_ref())?;
        Ok(())
    }
}

fn number_from_id(id: &Id) -> Result<u64, PluginError> {
    match *id {
        Id::Num(n) => Ok(n as u64),
        Id::Str(ref s) => Ok(s.parse::<u64>()?),
        Id::None(_) => Err(PluginError::Other("id is not provided".to_string())),
    }
}

pub fn parse_stdin() -> Result<PluginServerRpc, PluginError> {
    let mut msg = String::new();
    std::io::stdin().read_line(&mut msg)?;
    let rpc = match JsonRpc::parse(&msg) {
        Ok(value @ JsonRpc::Request(_)) => {
            let m_id = value
                .get_id()
                .ok_or(PluginError::Other("request is missing id".to_string()))?;
            let id = number_from_id(&m_id)?;
            PluginServerRpc::Request {
                id,
                method: value
                    .get_method()
                    .ok_or(PluginError::Other("request is missing method".to_string()))?
                    .to_string(),
                params: serde_json::to_value(
                    value
                        .get_params()
                        .ok_or(PluginError::Other("request is missing params".to_string()))?,
                )?,
            }
        }
        Ok(value @ JsonRpc::Notification(_)) => PluginServerRpc::Notification {
            method: value
                .get_method()
                .ok_or(PluginError::Other(
                    "notification is missing method".to_string(),
                ))?
                .to_string(),
            params: serde_json::to_value(value.get_params().ok_or(PluginError::Other(
                "notification is missing params".to_string(),
            ))?)?,
        },
        o => {
            todo!("{:#?}", o)
        }
    };
    Ok(rpc)
}

pub fn object_from_stdin<T: DeserializeOwned>() -> Result<T, PluginError> {
    let mut json = String::new();
    std::io::stdin().read_line(&mut json)?;
    let result = serde_json::from_str(&json)?;
    Ok(result)
}

pub fn object_to_stdout(object: &impl Serialize) -> Result<(), PluginError> {
    println!("{}", serde_json::to_string(object)?);
    Ok(())
}

fn send_host_notification(method: &str, params: &Value) -> Result<(), PluginError> {
    object_to_stdout(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    }))?;
    unsafe { host_handle_rpc() };
    Ok(())
}

fn send_host_request(id: u64, method: &str, params: &Value) -> Result<(), PluginError> {
    object_to_stdout(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))?;
    unsafe { host_handle_rpc() };
    Ok(())
}

fn send_host_success(id: u64, result: &Value) -> Result<(), PluginError> {
    object_to_stdout(&jsonrpc_lite::JsonRpc::success(id as i64, result))?;
    unsafe { host_handle_rpc() };
    Ok(())
}

fn send_host_error(id: u64, message: &str) -> Result<(), PluginError> {
    object_to_stdout(&jsonrpc_lite::JsonRpc::error(
        id as i64,
        jsonrpc_lite::Error {
            code: jsonrpc_lite::ErrorCode::InvalidParams.code(),
            message: message.to_string(),
            data: None,
        },
    ))?;
    unsafe { host_handle_rpc() };
    Ok(())
}

#[link(wasm_import_module = "lapce")]
extern "C" {
    fn host_handle_rpc();
    fn host_handle_stderr();
}
