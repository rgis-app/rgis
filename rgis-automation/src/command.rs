//! The command vocabulary shared by `--script`, the wasm `dispatch` export,
//! and the headless tests.
//!
//! A script is a JSON array of steps. Each step is an object whose `"cmd"`
//! field names the command; the remaining fields are its parameters:
//!
//! ```json
//! [{"cmd": "load_file", "path": "countries.geojson"}, {"cmd": "wait_idle"}]
//! ```

use bevy::color::{Color, Srgba};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// Every command name, in the order they're documented.
pub const COMMAND_NAMES: &[&str] = &[
    "load_file",
    "load_url",
    "load_text",
    "change_crs",
    "toggle_visibility",
    "move_layer",
    "delete_layer",
    "duplicate_layer",
    "rename_layer",
    "set_fill_color",
    "set_stroke_color",
    "set_point_size",
    "center_on_layer",
    "pan",
    "zoom",
    "select_feature",
    "deselect",
    "open_window",
    "close_window",
    "set_animations",
    "wait_idle",
    "frames",
    "dump_state",
    "screenshot",
];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Read a file from disk and load it as a new layer. Native only.
    LoadFile {
        path: String,
        format: Option<Format>,
        crs: Option<CrsSpec>,
        name: Option<String>,
    },
    /// Fetch a file over HTTP(S) and load it as a new layer.
    LoadUrl {
        url: String,
        format: Option<Format>,
        crs: Option<CrsSpec>,
        name: Option<String>,
    },
    /// Load inline text (GeoJSON, WKT, GPX) as a new layer.
    LoadText {
        text: String,
        format: Format,
        crs: Option<CrsSpec>,
        name: Option<String>,
    },
    /// Change the map's target CRS. Exactly one of `epsg` or `proj`.
    ChangeCrs {
        epsg: Option<u16>,
        proj: Option<String>,
    },
    /// Toggle a layer's visibility, or set it when `visible` is given.
    ToggleVisibility {
        layer: LayerRef,
        visible: Option<bool>,
    },
    MoveLayer {
        layer: LayerRef,
        direction: Direction,
    },
    DeleteLayer {
        layer: LayerRef,
    },
    DuplicateLayer {
        layer: LayerRef,
    },
    RenameLayer {
        layer: LayerRef,
        name: String,
    },
    SetFillColor {
        layer: LayerRef,
        color: ColorSpec,
    },
    SetStrokeColor {
        layer: LayerRef,
        color: ColorSpec,
    },
    SetPointSize {
        layer: LayerRef,
        size: f32,
    },
    /// Fly the camera to a layer's extent.
    CenterOnLayer {
        layer: LayerRef,
    },
    /// Move the camera by screen pixels. Positive `dx` moves the view right
    /// (east), positive `dy` moves it up (north).
    Pan {
        #[serde(default)]
        dx: f32,
        #[serde(default)]
        dy: f32,
    },
    /// Zoom around the view center. `factor > 1` zooms in, `< 1` zooms out.
    Zoom {
        factor: f32,
    },
    /// Select a feature by its 0-based position in the layer's source file.
    SelectFeature {
        layer: LayerRef,
        feature: usize,
    },
    /// Clear the selection (and close the feature properties window).
    Deselect,
    OpenWindow {
        title: String,
    },
    CloseWindow {
        title: String,
    },
    /// Turn fade/fly animations on or off.
    SetAnimations {
        enabled: bool,
    },
    /// Wait until no background jobs, camera flights, or fades are running.
    WaitIdle {
        max_frames: Option<u32>,
        timeout_secs: Option<f64>,
    },
    /// Let `n` frames run before the next step.
    Frames {
        n: u32,
    },
    /// Write the current app state as JSON. Native only.
    DumpState {
        path: String,
    },
    /// Save a PNG of the primary window. Native only.
    Screenshot {
        path: String,
    },
}

impl Command {
    /// The command's `"cmd"` name.
    pub fn name(&self) -> &'static str {
        match self {
            Command::LoadFile { .. } => "load_file",
            Command::LoadUrl { .. } => "load_url",
            Command::LoadText { .. } => "load_text",
            Command::ChangeCrs { .. } => "change_crs",
            Command::ToggleVisibility { .. } => "toggle_visibility",
            Command::MoveLayer { .. } => "move_layer",
            Command::DeleteLayer { .. } => "delete_layer",
            Command::DuplicateLayer { .. } => "duplicate_layer",
            Command::RenameLayer { .. } => "rename_layer",
            Command::SetFillColor { .. } => "set_fill_color",
            Command::SetStrokeColor { .. } => "set_stroke_color",
            Command::SetPointSize { .. } => "set_point_size",
            Command::CenterOnLayer { .. } => "center_on_layer",
            Command::Pan { .. } => "pan",
            Command::Zoom { .. } => "zoom",
            Command::SelectFeature { .. } => "select_feature",
            Command::Deselect => "deselect",
            Command::OpenWindow { .. } => "open_window",
            Command::CloseWindow { .. } => "close_window",
            Command::SetAnimations { .. } => "set_animations",
            Command::WaitIdle { .. } => "wait_idle",
            Command::Frames { .. } => "frames",
            Command::DumpState { .. } => "dump_state",
            Command::Screenshot { .. } => "screenshot",
        }
    }

    /// Checks parameter values that serde can't, so bad scripts fail before
    /// anything runs.
    fn validate(&self) -> Result<(), String> {
        match self {
            Command::ChangeCrs { epsg, proj } => match (epsg, proj) {
                (Some(_), None) | (None, Some(_)) => Ok(()),
                _ => Err("expected exactly one of `epsg` or `proj`".into()),
            },
            Command::SetPointSize { size, .. } if !(size.is_finite() && *size > 0.) => {
                Err(format!("`size` must be a positive number, got {size}"))
            }
            Command::Pan { dx, dy } if !(dx.is_finite() && dy.is_finite()) => {
                Err("`dx` and `dy` must be finite numbers".into())
            }
            Command::Zoom { factor } if !(factor.is_finite() && *factor > 0.) => {
                Err(format!("`factor` must be a positive number, got {factor}"))
            }
            Command::WaitIdle {
                timeout_secs: Some(timeout),
                ..
            } if !(timeout.is_finite() && *timeout > 0.) => Err(format!(
                "`timeout_secs` must be a positive number, got {timeout}"
            )),
            Command::Frames { n } if *n > MAX_FRAMES => {
                Err(format!("`n` must be at most {MAX_FRAMES}, got {n}"))
            }
            _ => Ok(()),
        }
    }
}

const MAX_FRAMES: u32 = 1_000_000;

/// Parse a script: a JSON array of steps, or a single step object.
pub fn parse_script(json: &str) -> Result<Vec<Command>, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("script is not valid JSON: {e}"))?;
    match value {
        Value::Array(steps) => steps
            .into_iter()
            .enumerate()
            .map(|(i, step)| parse_step(step).map_err(|e| format!("step {}: {e}", i + 1)))
            .collect(),
        step @ Value::Object(_) => Ok(vec![parse_step(step)?]),
        _ => Err("script must be a JSON array of steps like [{\"cmd\": \"wait_idle\"}]".into()),
    }
}

/// Parse a single `{"cmd": ..., ...}` step.
pub fn parse_step(step: Value) -> Result<Command, String> {
    let Value::Object(ref fields) = step else {
        return Err(format!(
            "expected an object like {{\"cmd\": \"wait_idle\"}}, got {step}"
        ));
    };
    let name = match fields.get("cmd") {
        Some(Value::String(name)) => name.clone(),
        Some(other) => return Err(format!("`cmd` must be a string, got {other}")),
        None => return Err("missing `cmd` field".into()),
    };
    if !COMMAND_NAMES.contains(&name.as_str()) {
        return Err(format!(
            "unknown command \"{name}\"; known commands: {}",
            COMMAND_NAMES.join(", ")
        ));
    }
    let command = Command::deserialize(step).map_err(|e| format!("{name}: {e}"))?;
    command.validate().map_err(|e| format!("{name}: {e}"))?;
    Ok(command)
}

/// A reference to a layer: its name, its position in the draw order, or its
/// internal id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerRef {
    /// Position in draw order: 0 is the bottom layer, -1 the top one.
    Index(i64),
    /// Exact layer name. Fails if several layers share it.
    Name(String),
    /// Internal layer id, as reported in the state's `layers[].id`.
    Id(u16),
}

impl std::fmt::Display for LayerRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayerRef::Index(index) => write!(f, "index {index}"),
            LayerRef::Name(name) => write!(f, "\"{name}\""),
            LayerRef::Id(id) => write!(f, "id {id}"),
        }
    }
}

impl<'de> Deserialize<'de> for LayerRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        const EXPECTED: &str = "expected a layer name (string), a draw-order index \
            (integer; 0 is the bottom layer, -1 the top), or {\"id\": <layer id>}";
        match Value::deserialize(deserializer)? {
            Value::Number(n) => n
                .as_i64()
                .map(LayerRef::Index)
                .ok_or_else(|| D::Error::custom(EXPECTED)),
            Value::String(name) => Ok(LayerRef::Name(name)),
            Value::Object(fields) if fields.len() == 1 => fields
                .get("id")
                .and_then(Value::as_u64)
                .and_then(|id| u16::try_from(id).ok())
                .filter(|id| *id != 0)
                .map(LayerRef::Id)
                .ok_or_else(|| D::Error::custom(EXPECTED)),
            _ => Err(D::Error::custom(EXPECTED)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Up,
    Down,
}

impl From<Direction> for rgis_events::MoveDirection {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Up => rgis_events::MoveDirection::Up,
            Direction::Down => rgis_events::MoveDirection::Down,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[serde(alias = "json")]
    Geojson,
    #[serde(alias = "shp")]
    Shapefile,
    Wkt,
    Gpx,
    #[serde(alias = "tif", alias = "tiff")]
    Geotiff,
}

impl Format {
    /// Guess the format from a path or URL's extension.
    pub fn from_path(path: &str) -> Option<Format> {
        let path = path.split(['?', '#']).next().unwrap_or(path);
        let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
        match extension.as_str() {
            "geojson" | "json" => Some(Format::Geojson),
            "shp" => Some(Format::Shapefile),
            "wkt" => Some(Format::Wkt),
            "gpx" => Some(Format::Gpx),
            "tif" | "tiff" => Some(Format::Geotiff),
            _ => None,
        }
    }
}

impl From<Format> for geo_file_loader::FileFormat {
    fn from(format: Format) -> Self {
        match format {
            Format::Geojson => geo_file_loader::FileFormat::GeoJson,
            Format::Shapefile => geo_file_loader::FileFormat::Shapefile,
            Format::Wkt => geo_file_loader::FileFormat::Wkt,
            Format::Gpx => geo_file_loader::FileFormat::Gpx,
            Format::Geotiff => geo_file_loader::FileFormat::GeoTiff,
        }
    }
}

/// A source CRS for loaded data: `4326`, `"EPSG:4326"`, or a PROJ string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrsSpec {
    Epsg(u16),
    Proj(String),
}

impl<'de> Deserialize<'de> for CrsSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        const EXPECTED: &str =
            "expected an EPSG code (4326 or \"EPSG:4326\") or a PROJ string (\"+proj=...\")";
        match Value::deserialize(deserializer)? {
            Value::Number(n) => n
                .as_u64()
                .and_then(|code| u16::try_from(code).ok())
                .map(CrsSpec::Epsg)
                .ok_or_else(|| D::Error::custom(EXPECTED)),
            Value::String(s) => {
                let s = s.trim();
                let code = s
                    .strip_prefix("EPSG:")
                    .or_else(|| s.strip_prefix("epsg:"))
                    .unwrap_or(s);
                if let Ok(code) = code.parse::<u16>() {
                    Ok(CrsSpec::Epsg(code))
                } else if s.starts_with('+') {
                    Ok(CrsSpec::Proj(s.to_string()))
                } else {
                    Err(D::Error::custom(EXPECTED))
                }
            }
            _ => Err(D::Error::custom(EXPECTED)),
        }
    }
}

/// A color: `"#rrggbb"`, `"#rrggbbaa"`, or `[r, g, b]` / `[r, g, b, a]` with
/// sRGB components in `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorSpec(pub Color);

impl<'de> Deserialize<'de> for ColorSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        const EXPECTED: &str = "expected \"#rrggbb\", \"#rrggbbaa\", or [r, g, b(, a)] with \
            components between 0 and 1";
        match Value::deserialize(deserializer)? {
            Value::String(hex) => Srgba::hex(hex.trim())
                .map(|srgba| ColorSpec(srgba.into()))
                .map_err(|_| D::Error::custom(EXPECTED)),
            Value::Array(components) => {
                let components = components
                    .iter()
                    .map(|c| {
                        c.as_f64()
                            .filter(|c| (0.0..=1.0).contains(c))
                            .map(|c| c as f32)
                    })
                    .collect::<Option<Vec<f32>>>()
                    .ok_or_else(|| D::Error::custom(EXPECTED))?;
                match *components.as_slice() {
                    [r, g, b] => Ok(ColorSpec(Color::srgb(r, g, b))),
                    [r, g, b, a] => Ok(ColorSpec(Color::srgba(r, g, b, a))),
                    _ => Err(D::Error::custom(EXPECTED)),
                }
            }
            _ => Err(D::Error::custom(EXPECTED)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_err(json: &str) -> String {
        match parse_script(json) {
            Ok(commands) => panic!("expected an error, got {commands:?}"),
            Err(e) => e,
        }
    }

    #[test]
    fn parses_a_script() {
        let script = parse_script(
            r##"[
                {"cmd": "load_file", "path": "a.geojson"},
                {"cmd": "change_crs", "epsg": 3857},
                {"cmd": "toggle_visibility", "layer": "a"},
                {"cmd": "move_layer", "layer": -1, "direction": "down"},
                {"cmd": "set_fill_color", "layer": {"id": 3}, "color": "#ff000080"},
                {"cmd": "deselect"},
                {"cmd": "wait_idle"}
            ]"##,
        );
        assert_eq!(
            script,
            Ok(vec![
                Command::LoadFile {
                    path: "a.geojson".into(),
                    format: None,
                    crs: None,
                    name: None,
                },
                Command::ChangeCrs {
                    epsg: Some(3857),
                    proj: None,
                },
                Command::ToggleVisibility {
                    layer: LayerRef::Name("a".into()),
                    visible: None,
                },
                Command::MoveLayer {
                    layer: LayerRef::Index(-1),
                    direction: Direction::Down,
                },
                Command::SetFillColor {
                    layer: LayerRef::Id(3),
                    color: ColorSpec(Color::srgba_u8(255, 0, 0, 128)),
                },
                Command::Deselect,
                Command::WaitIdle {
                    max_frames: None,
                    timeout_secs: None,
                },
            ])
        );
    }

    #[test]
    fn accepts_a_single_step() {
        assert_eq!(
            parse_script(r#"{"cmd": "frames", "n": 2}"#),
            Ok(vec![Command::Frames { n: 2 }])
        );
    }

    #[test]
    fn reports_unknown_commands() {
        let e = parse_err(r#"[{"cmd": "wait_idle"}, {"cmd": "explode"}]"#);
        assert!(e.starts_with("step 2: unknown command \"explode\""), "{e}");
    }

    #[test]
    fn reports_bad_params() {
        let e = parse_err(r#"[{"cmd": "load_file"}]"#);
        assert!(e.contains("step 1: load_file: missing field `path`"), "{e}");

        let e = parse_err(r#"[{"cmd": "load_file", "pth": "x"}]"#);
        assert!(e.contains("unknown field `pth`"), "{e}");

        let e = parse_err(r#"[{"cmd": "change_crs"}]"#);
        assert!(e.contains("exactly one of `epsg` or `proj`"), "{e}");

        let e = parse_err(r#"[{"cmd": "zoom", "factor": -2}]"#);
        assert!(e.contains("`factor` must be a positive number"), "{e}");

        let e = parse_err(r#"[{"cmd": "toggle_visibility", "layer": true}]"#);
        assert!(e.contains("expected a layer name"), "{e}");

        let e = parse_err(r#"[{"cmd": "set_fill_color", "layer": 0, "color": [2, 0, 0]}]"#);
        assert!(e.contains("components between 0 and 1"), "{e}");

        let e = parse_err(r#"[{"cmd": 1}]"#);
        assert!(e.contains("`cmd` must be a string"), "{e}");

        let e = parse_err("[1]");
        assert!(e.contains("expected an object"), "{e}");

        let e = parse_err("not json");
        assert!(e.contains("not valid JSON"), "{e}");
    }

    #[test]
    fn every_command_name_parses() {
        // Keeps COMMAND_NAMES in sync with the enum.
        for name in COMMAND_NAMES {
            let e = parse_script(&format!(r#"{{"cmd": "{name}", "bogus": 1}}"#));
            let e = e.err().unwrap_or_default();
            assert!(!e.contains("unknown command"), "{name}: {e}");
        }
    }

    #[test]
    fn crs_spec() {
        let parse = |json: &str| {
            parse_script(&format!(
                r#"{{"cmd": "load_url", "url": "u", "crs": {json}}}"#
            ))
        };
        let crs_of = |json: &str| match parse(json) {
            Ok(commands) => match commands.first() {
                Some(Command::LoadUrl { crs, .. }) => crs.clone(),
                _ => None,
            },
            Err(_) => None,
        };
        assert_eq!(crs_of("4326"), Some(CrsSpec::Epsg(4326)));
        assert_eq!(crs_of(r#""EPSG:3857""#), Some(CrsSpec::Epsg(3857)));
        assert_eq!(
            crs_of(r#""+proj=longlat +datum=WGS84""#),
            Some(CrsSpec::Proj("+proj=longlat +datum=WGS84".into()))
        );
        assert!(parse(r#""WGS84""#).is_err());
        assert!(parse("70000").is_err());
    }

    #[test]
    fn format_from_path() {
        assert_eq!(Format::from_path("a/b.GeoJSON"), Some(Format::Geojson));
        assert_eq!(Format::from_path("x.tif?raw=1"), Some(Format::Geotiff));
        assert_eq!(Format::from_path("x.csv"), None);
        assert_eq!(Format::from_path("noext"), None);
    }
}
