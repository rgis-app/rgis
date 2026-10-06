use clap::{Arg, ArgAction, Command};
use std::path::PathBuf;

pub struct CliArgs {
    pub url: Option<String>,
    /// Raw `--script` value: a JSON array of steps, or `@path` to a file containing one.
    pub script: Option<String>,
    pub screenshot: Option<PathBuf>,
    pub dump_state: Option<PathBuf>,
    pub exit_after_script: bool,
    pub show_window: bool,
    pub window_size: Option<(u32, u32)>,
}

impl CliArgs {
    /// Whether this is a batch run that should exit once the script is done.
    pub fn is_batch(&self) -> bool {
        self.exit_after_script || self.screenshot.is_some() || self.dump_state.is_some()
    }
}

pub fn run() -> Result<CliArgs, String> {
    let matches = Command::new("rgis")
        .author("Corey Farwell <coreyf@rwell.org>")
        .about("Geospatial data viewer written in Rust")
        .arg(
            Arg::new("url")
                .long("url")
                .help("URL to a GeoJSON file to load on startup")
                .value_name("URL"),
        )
        .arg(
            Arg::new("script")
                .long("script")
                .help(
                    "Steps to run after startup: a JSON array of {\"cmd\": ..., ...params} \
                     objects, or @path to a file containing one. See docs/automation.md",
                )
                .value_name("JSON|@FILE"),
        )
        .arg(
            Arg::new("screenshot")
                .long("screenshot")
                .help("Save a PNG of the window after the script finishes, then exit")
                .value_name("PATH")
                .value_parser(clap::value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("dump-state")
                .long("dump-state")
                .help("Write the app state as JSON after the script finishes, then exit")
                .value_name("PATH")
                .value_parser(clap::value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("exit-after-script")
                .long("exit-after-script")
                .help("Exit once the script finishes (implied by --screenshot and --dump-state)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("show-window")
                .long("show-window")
                .help(
                    "Show the window during batch runs (hidden by default so rendering \
                     doesn't stall when it's covered by other windows)",
                )
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("window-size")
                .long("window-size")
                .help("Window size in logical pixels, e.g. 1280x720 (the default for batch runs)")
                .value_name("WIDTHxHEIGHT")
                .value_parser(parse_window_size),
        )
        .get_matches();

    Ok(CliArgs {
        url: matches.get_one::<String>("url").cloned(),
        script: matches.get_one::<String>("script").cloned(),
        screenshot: matches.get_one::<PathBuf>("screenshot").cloned(),
        dump_state: matches.get_one::<PathBuf>("dump-state").cloned(),
        exit_after_script: matches.get_flag("exit-after-script"),
        show_window: matches.get_flag("show-window"),
        window_size: matches.get_one::<(u32, u32)>("window-size").copied(),
    })
}

fn parse_window_size(value: &str) -> Result<(u32, u32), String> {
    let error = || format!("expected WIDTHxHEIGHT (e.g. 1280x720), got \"{value}\"");
    let (width, height) = value.split_once('x').ok_or_else(error)?;
    let width: u32 = width.trim().parse().map_err(|_| error())?;
    let height: u32 = height.trim().parse().map_err(|_| error())?;
    if width == 0 || height == 0 {
        return Err(error());
    }
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::parse_window_size;

    #[test]
    fn window_size() {
        assert_eq!(parse_window_size("1280x720"), Ok((1280, 720)));
        assert!(parse_window_size("1280").is_err());
        assert!(parse_window_size("0x720").is_err());
        assert!(parse_window_size("axb").is_err());
    }
}
