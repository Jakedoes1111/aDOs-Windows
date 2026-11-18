//! aDOs's MCP server (crate `ados-mcp`): started at launch so every agent session aDOs
//! starts can act in aDOs. The same binary is the stdio shim agents run: `ados mcp-bridge`.
use gpui_kit::App;

/// `ados mcp-bridge`: run as the shim and exit, before any window opens.
pub fn bridge_mode() {
    if std::env::args().nth(1).as_deref() == Some("mcp-bridge") {
        std::process::exit(ados_mcp::bridge::run());
    }
}

pub fn start(cx: &mut App) {
    let hooks = ados_mcp::Hooks {
        current_layout: || ados::storage::working().ok().flatten().map(|w| w.layout.name),
    };
    if let Err(e) = ados_mcp::start(hooks, cx) {
        // Without it the assistant still works, just without aDOs tools.
        eprintln!("aDOs MCP server did not start: {e:#}");
    }
}
