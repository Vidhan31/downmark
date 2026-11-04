//! Command-line argument parsing and configuration.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{settings::Layout, workspace::GlanceStart};

/// Where to scroll or focus when opening a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Target {
    /// Jump cursor and view to a 1-based line number.
    Line(usize),
    /// Jump to a heading matching the text or slug.
    Heading(String),
}

/// A file path to open with an optional jump target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileTarget {
    /// The path to the file.
    pub(crate) path: PathBuf,
    /// Optional line or heading to reveal on open.
    pub(crate) target: Option<Target>,
}

/// Parsed options for launching the application window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LaunchConfig {
    /// Documents to open.
    pub(crate) files: Vec<FileTarget>,
    /// Explicit layout requested from the CLI, if any.
    pub(crate) layout: Option<Layout>,
    /// Whether to start in glance mode.
    pub(crate) glance: GlanceStart,
    /// Whether to open in a fresh window instead of reusing an existing instance.
    pub(crate) new_window: bool,
}

/// The outcome of parsing command-line arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliAction {
    /// Launch the GUI with the given configuration.
    Run(LaunchConfig),
    /// Print help text and exit.
    Help,
    /// Print version text and exit.
    Version,
}

/// Help text displayed for `--help`.
pub(crate) const HELP_TEXT: &str = "\
Downmark — a fast, native Markdown editor and previewer.

Usage: downmark [OPTIONS] [FILE[:LINE]]...

Arguments:
  [FILE]...             Files to open (supports path:line or path:line:col syntax)

Options:
  -l, --line <NUM>      Jump to line number in the opened document
  -H, --heading <TEXT>  Jump to heading in the opened document
  -p, --preview         Open in preview-only view
  -e, --editor          Open in editor-only view
      --split           Open in side-by-side view
  -g, --glance          Open in lightweight glance mode
  -n, --new-window      Open in a new window instead of existing instance
  -h, --help            Print help
  -V, --version         Print version
";

/// Parses command-line arguments into a [`CliAction`].
///
/// # Errors
///
/// Returns an error message if an argument is invalid or missing a value.
#[expect(
    clippy::too_many_lines,
    reason = "Complete CLI argument parsing loop with options and positional parsing"
)]
pub(crate) fn parse_args<I>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut files_raw: Vec<String> = Vec::new();
    let mut layout: Option<Layout> = None;
    let mut glance = GlanceStart::Normal;
    let mut new_window = false;
    let mut line_target: Option<usize> = None;
    let mut heading_target: Option<String> = None;

    let args: Vec<String> = args
        .into_iter()
        .map(|os| os.to_string_lossy().into_owned())
        .collect();
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == "-h" || arg == "--help" {
            return Ok(CliAction::Help);
        }
        if arg == "-V" || arg == "--version" {
            return Ok(CliAction::Version);
        }
        if arg == "-n" || arg == "--new-window" {
            new_window = true;
        } else if arg == "-p" || arg == "--preview" {
            layout = Some(Layout::Preview);
        } else if arg == "-e" || arg == "--editor" {
            layout = Some(Layout::Editor);
        } else if arg == "--split" {
            layout = Some(Layout::Split);
        } else if arg == "-g" || arg == "--glance" {
            glance = GlanceStart::Glance;
        } else if arg == "-l" || arg == "--line" {
            let val = iter
                .next()
                .ok_or_else(|| format!("option '{arg}' requires an argument"))?;
            let line = val
                .parse::<usize>()
                .map_err(|_| format!("invalid line number '{val}': must be a positive integer"))?;
            if line == 0 {
                return Err("invalid line number '0': line numbers start at 1".to_string());
            }
            line_target = Some(line);
        } else if let Some(val) = arg.strip_prefix("--line=") {
            let line = val
                .parse::<usize>()
                .map_err(|_| format!("invalid line number '{val}': must be a positive integer"))?;
            if line == 0 {
                return Err("invalid line number '0': line numbers start at 1".to_string());
            }
            line_target = Some(line);
        } else if arg.starts_with("-l") && arg.len() > 2 {
            let val = &arg[2..];
            let line = val
                .parse::<usize>()
                .map_err(|_| format!("invalid line number '{val}': must be a positive integer"))?;
            if line == 0 {
                return Err("invalid line number '0': line numbers start at 1".to_string());
            }
            line_target = Some(line);
        } else if arg == "-H" || arg == "--heading" {
            let val = iter
                .next()
                .ok_or_else(|| format!("option '{arg}' requires an argument"))?;
            heading_target = Some(val);
        } else if let Some(val) = arg.strip_prefix("--heading=") {
            heading_target = Some(val.to_string());
        } else if arg.starts_with('+')
            && arg.len() > 1
            && arg[1..].chars().all(|c| c.is_ascii_digit())
        {
            let val = &arg[1..];
            let line = val
                .parse::<usize>()
                .map_err(|_| format!("invalid line number '{val}': must be a positive integer"))?;
            if line == 0 {
                return Err("invalid line number '0': line numbers start at 1".to_string());
            }
            line_target = Some(line);
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!(
                "unrecognized option '{arg}'\nTry 'downmark --help' for more information."
            ));
        } else {
            files_raw.push(arg);
        }
    }

    let default_target = match (line_target, heading_target) {
        (Some(line), _) => Some(Target::Line(line)),
        (None, Some(heading)) => Some(Target::Heading(heading)),
        (None, None) => None,
    };

    let mut files = Vec::with_capacity(files_raw.len());
    let mut default_applied = false;

    for raw in files_raw {
        let (path, target) = parse_file_and_target(&raw);
        let target = if let Some(t) = target {
            Some(t)
        } else if !default_applied {
            default_applied = true;
            default_target.clone()
        } else {
            None
        };
        let canonical = path.canonicalize().unwrap_or(path);
        files.push(FileTarget {
            path: canonical,
            target,
        });
    }

    Ok(CliAction::Run(LaunchConfig {
        files,
        layout,
        glance,
        new_window,
    }))
}

/// Parses a string that might be `path:line` or `path:line:col`.
fn parse_file_and_target(arg: &str) -> (PathBuf, Option<Target>) {
    if Path::new(arg).exists() {
        return (PathBuf::from(arg), None);
    }

    if let Some((rest, suffix)) = arg.rsplit_once(':')
        && let Ok(num) = suffix.parse::<usize>()
        && num > 0
    {
        // Check if rest also has :line (e.g. file:line:col where suffix is col)
        if let Some((path_part, line_part)) = rest.rsplit_once(':')
            && let Ok(line) = line_part.parse::<usize>()
            && line > 0
        {
            return (PathBuf::from(path_part), Some(Target::Line(line)));
        }
        return (PathBuf::from(rest), Some(Target::Line(num)));
    }

    (PathBuf::from(arg), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_help_and_version() {
        assert_eq!(parse_args([OsString::from("--help")]), Ok(CliAction::Help));
        assert_eq!(parse_args([OsString::from("-h")]), Ok(CliAction::Help));
        assert_eq!(
            parse_args([OsString::from("--version")]),
            Ok(CliAction::Version)
        );
        assert_eq!(parse_args([OsString::from("-V")]), Ok(CliAction::Version));
    }

    #[test]
    fn parses_view_modes() {
        let parsed = parse_args([OsString::from("--preview")]).expect("valid parse");
        assert_eq!(
            parsed,
            CliAction::Run(LaunchConfig {
                files: Vec::new(),
                layout: Some(Layout::Preview),
                glance: GlanceStart::Normal,
                new_window: false,
            })
        );

        let parsed = parse_args([OsString::from("-e")]).expect("valid parse");
        assert_eq!(
            parsed,
            CliAction::Run(LaunchConfig {
                files: Vec::new(),
                layout: Some(Layout::Editor),
                glance: GlanceStart::Normal,
                new_window: false,
            })
        );

        let parsed = parse_args([OsString::from("--split")]).expect("valid parse");
        assert_eq!(
            parsed,
            CliAction::Run(LaunchConfig {
                files: Vec::new(),
                layout: Some(Layout::Split),
                glance: GlanceStart::Normal,
                new_window: false,
            })
        );

        let parsed = parse_args([OsString::from("-g")]).expect("valid parse");
        assert_eq!(
            parsed,
            CliAction::Run(LaunchConfig {
                files: Vec::new(),
                layout: None,
                glance: GlanceStart::Glance,
                new_window: false,
            })
        );
    }

    #[test]
    fn parses_new_window_flag() {
        let parsed = parse_args([OsString::from("--new-window")]).expect("valid parse");
        assert_eq!(
            parsed,
            CliAction::Run(LaunchConfig {
                files: Vec::new(),
                layout: None,
                glance: GlanceStart::Normal,
                new_window: true,
            })
        );

        let parsed = parse_args([OsString::from("-n")]).expect("valid parse");
        assert_eq!(
            parsed,
            CliAction::Run(LaunchConfig {
                files: Vec::new(),
                layout: None,
                glance: GlanceStart::Normal,
                new_window: true,
            })
        );
    }

    #[test]
    fn parses_line_flags_and_plus() {
        let parsed = parse_args([
            OsString::from("-l"),
            OsString::from("42"),
            OsString::from("test.md"),
        ])
        .expect("valid parse");
        match parsed {
            CliAction::Run(cfg) => {
                assert_eq!(cfg.files.len(), 1);
                assert_eq!(cfg.files[0].path, PathBuf::from("test.md"));
                assert_eq!(cfg.files[0].target, Some(Target::Line(42)));
            }
            _ => panic!("expected Run action"),
        }

        let parsed =
            parse_args([OsString::from("+15"), OsString::from("test.md")]).expect("valid parse");
        match parsed {
            CliAction::Run(cfg) => {
                assert_eq!(cfg.files[0].target, Some(Target::Line(15)));
            }
            _ => panic!("expected Run action"),
        }

        let parsed =
            parse_args([OsString::from("-l99"), OsString::from("test.md")]).expect("valid parse");
        match parsed {
            CliAction::Run(cfg) => {
                assert_eq!(cfg.files[0].target, Some(Target::Line(99)));
            }
            _ => panic!("expected Run action"),
        }

        let parsed = parse_args([OsString::from("--line=50"), OsString::from("test.md")])
            .expect("valid parse");
        match parsed {
            CliAction::Run(cfg) => {
                assert_eq!(cfg.files[0].target, Some(Target::Line(50)));
            }
            _ => panic!("expected Run action"),
        }
    }

    #[test]
    fn parses_heading_flag() {
        let parsed = parse_args([
            OsString::from("-H"),
            OsString::from("Quick Start"),
            OsString::from("test.md"),
        ])
        .expect("valid parse");
        match parsed {
            CliAction::Run(cfg) => {
                assert_eq!(
                    cfg.files[0].target,
                    Some(Target::Heading("Quick Start".to_string()))
                );
            }
            _ => panic!("expected Run action"),
        }

        let parsed = parse_args([
            OsString::from("--heading=System Specs"),
            OsString::from("test.md"),
        ])
        .expect("valid parse");
        match parsed {
            CliAction::Run(cfg) => {
                assert_eq!(
                    cfg.files[0].target,
                    Some(Target::Heading("System Specs".to_string()))
                );
            }
            _ => panic!("expected Run action"),
        }
    }

    #[test]
    fn parses_multiple_files_and_targets() {
        let parsed = parse_args([
            OsString::from("file1.md:20"),
            OsString::from("file2.md"),
            OsString::from("file3.md:30:5"),
        ])
        .expect("valid parse");
        match parsed {
            CliAction::Run(cfg) => {
                assert_eq!(cfg.files.len(), 3);
                assert_eq!(cfg.files[0].path, PathBuf::from("file1.md"));
                assert_eq!(cfg.files[0].target, Some(Target::Line(20)));
                assert_eq!(cfg.files[1].path, PathBuf::from("file2.md"));
                assert_eq!(cfg.files[1].target, None);
                assert_eq!(cfg.files[2].path, PathBuf::from("file3.md"));
                assert_eq!(cfg.files[2].target, Some(Target::Line(30)));
            }
            _ => panic!("expected Run action"),
        }
    }

    #[test]
    fn parses_empty_arguments() {
        let parsed = parse_args(Vec::<OsString>::new()).expect("valid parse");
        assert_eq!(
            parsed,
            CliAction::Run(LaunchConfig {
                files: Vec::new(),
                layout: None,
                glance: GlanceStart::Normal,
                new_window: false,
            })
        );
    }

    #[test]
    fn parses_file_colon_line_syntax() {
        let (path, target) = parse_file_and_target("src/main.rs:125");
        assert_eq!(path, PathBuf::from("src/main.rs"));
        assert_eq!(target, Some(Target::Line(125)));

        let (path, target) = parse_file_and_target("src/main.rs:125:10");
        assert_eq!(path, PathBuf::from("src/main.rs"));
        assert_eq!(target, Some(Target::Line(125)));
    }

    #[test]
    fn reports_errors_for_unknown_flags_and_bad_lines() {
        assert!(parse_args([OsString::from("--unknown")]).is_err());
        assert!(parse_args([OsString::from("-l"), OsString::from("abc")]).is_err());
        assert!(parse_args([OsString::from("-l"), OsString::from("0")]).is_err());
        assert!(parse_args([OsString::from("-l")]).is_err());
        assert!(parse_args([OsString::from("-H")]).is_err());
    }
}
