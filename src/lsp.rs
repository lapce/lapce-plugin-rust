use psp_types::LspId;
use serde::{de::DeserializeOwned, Serialize};

use crate::{PluginError, PLUGIN_RPC};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LspRef {
    pub id: LspId,
}
impl LspRef {
    pub fn new(id: LspId) -> Self {
        Self { id }
    }

    pub fn send_request_blocking<P: Serialize, D: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<D, PluginError> {
        PLUGIN_RPC.lsp_send_request_blocking(self.id, method, params)
    }

    pub fn send_notification<P: Serialize>(
        &self,
        method: &str,
        params: P,
    ) -> Result<(), PluginError> {
        PLUGIN_RPC.lsp_send_notification(self.id, method, params)
    }
}
