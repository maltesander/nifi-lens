//! Command-line argument parsing via clap derive.

use std::path::PathBuf;

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Override the config file path (default: $XDG_CONFIG_HOME/nifilens/config.toml)
    #[arg(long, value_name = "PATH", global = true)]
    pub config: Option<PathBuf>,

    /// Override the active context from the config file
    #[arg(long, value_name = "NAME", global = true)]
    pub context: Option<String>,

    /// Raise log level to debug (shorthand for --log-level debug)
    #[arg(long, global = true)]
    pub debug: bool,

    /// Explicit log level (off, error, warn, info, debug, trace)
    #[arg(long, value_name = "LEVEL", value_enum, global = true)]
    pub log_level: Option<LogLevel>,

    /// Disable ANSI colors everywhere (stderr + TUI)
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Reserved for v2; currently errors immediately with "write mode not implemented"
    #[arg(long, global = true)]
    pub allow_writes: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Configuration file helpers.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Print version information (nifilens + nifi-rust-client).
    Version,
}

#[derive(clap::Subcommand, Debug)]
pub enum ConfigAction {
    /// Write a commented template to ~/.config/nifilens/config.toml (chmod 0600).
    Init {
        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,
    },
    /// Parse the config file and report errors without starting the TUI.
    Validate,
}

#[derive(clap::ValueEnum, Copy, Clone, Debug, PartialEq, Eq)]
#[value(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn as_tracing_filter(self) -> &'static str {
        match self {
            LogLevel::Off => "off",
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}
