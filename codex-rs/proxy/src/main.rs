use clap::Parser;
use codex_arg0::arg0_dispatch_or_else;

#[derive(Parser, Debug, Clone)]
#[command(name = "codex-proxy", about = "OpenAI-compatible HTTP passthrough API")]
struct Cli {
    #[command(flatten)]
    proxy: codex_proxy::ProxyCommand,
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|_codex_linux_sandbox_exe| async move {
        let cli = Cli::parse();
        codex_proxy::run(cli.proxy).await
    })
}
