//! Android JNI bridge (`libzalocore.so`) exposing `NodeRuntime.startServer` and `NodeRuntime.stopServer`.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

use jni::objects::{JClass, JString};
use jni::sys::jint;
use jni::JNIEnv;
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;
use zalo_server::config::Config;
use zalo_server::log::RotatingFileLogger;

static LOGGER_INIT: Once = Once::new();
static SHUTDOWN_TX: Mutex<Option<oneshot::Sender<()>>> = Mutex::new(None);
static ACTIVE_LOG_FILE: Mutex<Option<Arc<RotatingFileLogger>>> = Mutex::new(None);

fn init_logging() {
    LOGGER_INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_tag("ZalooCore")
                .with_max_level(log::LevelFilter::Info),
        );
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(AndroidLogMakeWriter)
            .try_init();
    });
}

struct AndroidLogMakeWriter;

struct AndroidLogWriter {
    buf: Vec<u8>,
}

impl Write for AndroidLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for AndroidLogWriter {
    fn drop(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        if let Ok(guard) = ACTIVE_LOG_FILE.lock() {
            if let Some(ref logger) = *guard {
                logger.write_raw(&self.buf);
            }
        }
        let text = String::from_utf8_lossy(&self.buf);
        let trimmed = text.trim_end();
        if !trimmed.is_empty() {
            log::info!("{trimmed}");
        }
    }
}

impl<'a> MakeWriter<'a> for AndroidLogMakeWriter {
    type Writer = AndroidLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        AndroidLogWriter { buf: Vec::new() }
    }
}

fn read_jstring(env: &mut JNIEnv, s: &JString) -> Result<String, String> {
    if s.is_null() {
        return Ok(String::new());
    }
    env.get_string(s)
        .map(|js| js.into())
        .map_err(|e| format!("Failed to read JNI string: {e}"))
}

fn record_error(file_logger: &Option<Arc<RotatingFileLogger>>, msg: &str) {
    log::error!("{msg}");
    if let Some(ref logger) = file_logger {
        logger.log("error", msg);
    }
}

#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_NodeRuntime_startServer(
    mut env: JNIEnv,
    _class: JClass,
    host: JString,
    port: jint,
    token: JString,
    data_dir: JString,
    log_file: JString,
) -> jint {
    init_logging();

    let log_file_str = read_jstring(&mut env, &log_file).unwrap_or_default();
    let log_file_path = {
        let trimmed = log_file_str.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    };
    let file_logger = log_file_path
        .as_ref()
        .map(|p| Arc::new(RotatingFileLogger::new(p)));

    if let Ok(mut guard) = ACTIVE_LOG_FILE.lock() {
        *guard = file_logger.clone();
    }

    let host_str = match read_jstring(&mut env, &host) {
        Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
        Ok(_) => "127.0.0.1".to_string(),
        Err(e) => {
            record_error(&file_logger, &e);
            return 1;
        }
    };
    let token_str = match read_jstring(&mut env, &token) {
        Ok(s) => s,
        Err(e) => {
            record_error(&file_logger, &e);
            return 1;
        }
    };
    let data_dir_str = match read_jstring(&mut env, &data_dir) {
        Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
        Ok(_) => {
            record_error(&file_logger, "dataDir must not be empty");
            return 1;
        }
        Err(e) => {
            record_error(&file_logger, &e);
            return 1;
        }
    };

    if !(1..=65535).contains(&port) {
        record_error(&file_logger, &format!("Invalid port: {port}"));
        return 1;
    }

    let access_token = {
        let trimmed = token_str.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };

    let config = Config {
        host: host_str,
        port: port as u16,
        data_dir: PathBuf::from(data_dir_str),
        access_token,
        read_receipts: true,
        log_file: log_file_path,
        public_dir: None,
    };

    if let Err(e) = config.validate() {
        record_error(&file_logger, &format!("Invalid server config: {e}"));
        return 1;
    }

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    if let Ok(mut guard) = SHUTDOWN_TX.lock() {
        if let Some(prev) = guard.replace(shutdown_tx) {
            let _ = prev.send(());
        }
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            record_error(&file_logger, &format!("Failed to build Tokio runtime: {e}"));
            if let Ok(mut guard) = SHUTDOWN_TX.lock() {
                *guard = None;
            }
            return 1;
        }
    };

    let result = runtime.block_on(async move {
        zalo_server::run_server(config, async move {
            let _ = shutdown_rx.await;
        })
        .await
    });

    if let Ok(mut guard) = SHUTDOWN_TX.lock() {
        *guard = None;
    }

    match result {
        Ok(()) => {
            log::info!("Zalỏ server stopped cleanly");
            if let Some(ref logger) = file_logger {
                logger.log("log", "Zalỏ server stopped cleanly");
            }
            0
        }
        Err(err) => {
            record_error(&file_logger, &format!("Zalỏ server error: {err}"));
            1
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_zalom3ui_app_server_NodeRuntime_stopServer(
    _env: JNIEnv,
    _class: JClass,
) {
    if let Ok(mut guard) = SHUTDOWN_TX.lock() {
        if let Some(tx) = guard.take() {
            let _ = tx.send(());
        }
    }
}
