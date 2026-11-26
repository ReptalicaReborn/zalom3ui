//! Standalone CLI binary (`zalom3ui-server`) running the Zalỏ HTTP/WebSocket server.

use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;

use tracing::{error, info};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;
use zalo_server::config::Config;
use zalo_server::log::RotatingFileLogger;

#[derive(Clone)]
struct TeeMakeWriter {
    file: Option<Arc<RotatingFileLogger>>,
}

struct TeeWriter {
    file: Option<Arc<RotatingFileLogger>>,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(buf);
        if let Some(ref logger) = self.file {
            logger.write_raw(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stderr().flush();
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for TeeMakeWriter {
    type Writer = TeeWriter;

    fn make_writer(&'a self) -> Self::Writer {
        TeeWriter {
            file: self.file.clone(),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Configuration error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let file_logger = config
        .log_file
        .as_ref()
        .map(|p| Arc::new(RotatingFileLogger::new(p)));

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(file_logger.is_none())
        .with_writer(TeeMakeWriter {
            file: file_logger.clone(),
        })
        .try_init();

    if let Err(err) = zalo_server::run_server(config, shutdown_signal()).await {
        error!("Zalỏ server exited with error: {err}");
        if let Some(ref logger) = file_logger {
            logger.log("error", &format!("Zalỏ server exited with error: {err}"));
        }
        return ExitCode::FAILURE;
    }

    info!("Zalỏ server stopped cleanly");
    ExitCode::SUCCESS
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sigterm) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sigterm.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received SIGINT, shutting down...");
        }
        _ = terminate => {
            info!("Received SIGTERM, shutting down...");
        }
    }
}
