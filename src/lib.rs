use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    pub kind: String,
    pub percent_used: f64,
    pub resets_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MeterFile {
    limits: Vec<Limit>,
}

pub fn parse_meter(raw: &str) -> Result<Vec<Limit>, serde_json::Error> {
    serde_json::from_str::<MeterFile>(raw).map(|file| file.limits)
}

pub fn meter_json(limits: &[Limit]) -> String {
    serde_json::to_string_pretty(&MeterFile { limits: limits.to_vec() }).unwrap_or_default()
}

#[derive(Debug, Deserialize)]
struct StatusWindow {
    used_percentage: f64,
    resets_at: f64,
}

#[derive(Debug, Default, Deserialize)]
struct RateLimits {
    five_hour: Option<StatusWindow>,
    seven_day: Option<StatusWindow>,
}

#[derive(Debug, Deserialize)]
struct StatusInput {
    #[serde(default)]
    rate_limits: Option<RateLimits>,
}

pub fn limits_from_statusline(raw: &str) -> Result<Vec<Limit>, serde_json::Error> {
    let input: StatusInput = serde_json::from_str(raw)?;
    let rate_limits = input.rate_limits.unwrap_or_default();
    Ok([("five_hour", rate_limits.five_hour), ("seven_day", rate_limits.seven_day)]
        .into_iter()
        .filter_map(|(kind, window)| {
            let window = window?;
            Some(Limit {
                kind: kind.into(),
                percent_used: window.used_percentage,
                resets_at: DateTime::from_timestamp(window.resets_at as i64, 0)?,
            })
        })
        .collect())
}

pub fn merge_limits(stored: &[Limit], fresh: &[Limit]) -> Vec<Limit> {
    let mut merged: Vec<Limit> = stored
        .iter()
        .filter(|old| !fresh.iter().any(|new| new.kind == old.kind))
        .cloned()
        .collect();
    for new in fresh {
        let kept = match stored.iter().find(|old| old.kind == new.kind) {
            Some(old) if old.resets_at > new.resets_at => old.clone(),
            Some(old) if old.resets_at == new.resets_at => Limit {
                percent_used: old.percent_used.max(new.percent_used),
                ..new.clone()
            },
            _ => new.clone(),
        };
        merged.push(kept);
    }
    merged.sort_by(|a, b| a.kind.cmp(&b.kind));
    merged
}

fn short_label(kind: &str) -> &str {
    match kind {
        "five_hour" => "5 h",
        "seven_day" => "sem",
        other => other,
    }
}

fn remaining_text(left: Duration) -> String {
    let minutes = left.num_minutes().max(0);
    let (days, hours, mins) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
    match (days, hours) {
        (0, 0) => format!("{mins} min"),
        (0, _) => format!("{hours} h {mins} min"),
        _ => format!("{days} d {hours} h"),
    }
}

const DEFAULT_CELLS: usize = 8;
const MIN_CELLS: usize = 6;
const MAX_CELLS: usize = 30;
const RESERVED_COLUMNS: usize = 10;
const SEPARATOR: &str = "  ·  ";
const CLAUDE_ORANGE: &str = "\x1b[38;2;215;119;87m";
const AMBER: &str = "\x1b[38;2;255;193;7m";
const RED: &str = "\x1b[38;2;255;107;128m";
const GRAY: &str = "\x1b[38;2;102;102;102m";
const RESET: &str = "\x1b[0m";

struct MeterPart {
    label: String,
    percent: f64,
    remaining: String,
}

impl MeterPart {
    fn suffix(&self, long: bool) -> String {
        if long {
            format!("(reinicia en {})", self.remaining)
        } else {
            format!("({})", self.remaining)
        }
    }

    fn text_width(&self, long: bool) -> usize {
        let percent = format!("{:.0} %", self.percent);
        self.label.chars().count() + percent.chars().count() + self.suffix(long).chars().count() + 3
    }

    fn render(&self, cells: usize, long: bool) -> String {
        let filled = ((self.percent / 100.0) * cells as f64).round() as usize;
        let color = match self.percent {
            p if p >= 90.0 => RED,
            p if p >= 70.0 => AMBER,
            _ => CLAUDE_ORANGE,
        };
        format!(
            "{} {}{}{}{}{} {:.0} %{RESET} {}",
            self.label,
            color,
            "▰".repeat(filled),
            GRAY,
            "▱".repeat(cells - filled),
            color,
            self.percent,
            self.suffix(long)
        )
    }
}

fn layout(parts: &[MeterPart], columns: Option<usize>) -> (usize, bool) {
    let Some(columns) = columns else {
        return (DEFAULT_CELLS, true);
    };
    let available = columns.saturating_sub(RESERVED_COLUMNS);
    let separators = SEPARATOR.chars().count() * (parts.len() - 1);
    let cells_for = |long: bool| {
        let text: usize = parts.iter().map(|part| part.text_width(long)).sum();
        available.saturating_sub(text + separators) / parts.len()
    };
    let long_cells = cells_for(true);
    if long_cells >= MIN_CELLS {
        return (long_cells.min(MAX_CELLS), true);
    }
    (cells_for(false).clamp(MIN_CELLS, MAX_CELLS), false)
}

pub fn render_meter(limits: &[Limit], now: DateTime<Utc>, columns: Option<usize>) -> String {
    let parts: Vec<MeterPart> = limits
        .iter()
        .filter(|limit| limit.resets_at > now)
        .map(|limit| MeterPart {
            label: short_label(&limit.kind).to_string(),
            percent: limit.percent_used.clamp(0.0, 100.0),
            remaining: remaining_text(limit.resets_at - now),
        })
        .collect();
    if parts.is_empty() {
        return "uso del plan: sin datos todavía".into();
    }
    let (cells, long) = layout(&parts, columns);
    parts
        .iter()
        .map(|part| part.render(cells, long))
        .collect::<Vec<_>>()
        .join(SEPARATOR)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneRef {
    pub pane_id: String,
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wrap {
    pub resume_at: DateTime<Utc>,
    pub kinds: Vec<String>,
    pub panes: Vec<PaneRef>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attempts: u32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub wrap: Option<Wrap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer_for: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    pub pane_id: String,
    pub agent: String,
    pub status: String,
    pub session: Option<String>,
}

pub fn wrap_up_targets(agents: &[Agent], event_pane: Option<&str>) -> Vec<PaneRef> {
    agents
        .iter()
        .filter(|agent| agent.agent == "claude")
        .filter(|agent| {
            let just_finished = Some(agent.pane_id.as_str()) == event_pane
                && matches!(agent.status.as_str(), "idle" | "done");
            agent.status == "working" || just_finished
        })
        .map(|agent| PaneRef {
            pane_id: agent.pane_id.clone(),
            session: agent.session.clone(),
        })
        .collect()
}

pub fn catch_up_targets(agents: &[Agent], event_pane: Option<&str>, wrap: &Wrap) -> Vec<PaneRef> {
    wrap_up_targets(agents, event_pane)
        .into_iter()
        .filter(|pane| wrap.panes.iter().all(|asked| asked.pane_id != pane.pane_id))
        .collect()
}

pub fn in_wrap_window(state: &State, now: DateTime<Utc>) -> bool {
    state
        .wrap
        .as_ref()
        .is_some_and(|wrap| wrap.attempts == 0 && now < wrap.resume_at)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Readiness {
    Ready,
    Skip,
    Unknown,
}

pub fn readiness(pane: &PaneRef, session: Option<&str>, status: &str) -> Readiness {
    if pane.session.as_deref() != session {
        Readiness::Skip
    } else if matches!(status, "idle" | "done") {
        Readiness::Ready
    } else {
        Readiness::Skip
    }
}

pub const RESUME_RETRY_MINUTES: i64 = 5;
pub const RESUME_ATTEMPTS: u32 = 3;

pub fn retry_resume(wrap: &Wrap, failed: Vec<PaneRef>, now: DateTime<Utc>) -> Option<Wrap> {
    let attempts = wrap.attempts + 1;
    (!failed.is_empty() && attempts < RESUME_ATTEMPTS).then(|| Wrap {
        resume_at: now + Duration::minutes(RESUME_RETRY_MINUTES),
        kinds: wrap.kinds.clone(),
        panes: failed,
        attempts,
    })
}

pub fn timer_needed(state: &State) -> Option<DateTime<Utc>> {
    let resume_at = state.wrap.as_ref()?.resume_at;
    (state.timer_for != Some(resume_at)).then_some(resume_at)
}

fn valid_session_name(name: &str) -> bool {
    !matches!(name, "" | "." | "..")
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub fn session_from_socket(socket: Option<&str>) -> String {
    socket
        .and_then(|path| path.strip_suffix("/herdr.sock"))
        .and_then(|dir| dir.rsplit_once('/'))
        .filter(|(parent, _)| parent.ends_with("/sessions"))
        .map(|(_, name)| name)
        .filter(|name| valid_session_name(name))
        .unwrap_or("default")
        .to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkState<'a> {
    Missing,
    Symlink(&'a Path),
    OtherFile,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkAction {
    Keep,
    Create,
    Replace,
    Blocked,
}

pub fn stable_link_action(current: LinkState, exe: &Path) -> LinkAction {
    match current {
        LinkState::Missing => LinkAction::Create,
        LinkState::Symlink(target) if target == exe => LinkAction::Keep,
        LinkState::Symlink(_) => LinkAction::Replace,
        LinkState::OtherFile => LinkAction::Blocked,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Nothing,
    WrapUp {
        resume_at: DateTime<Utc>,
        kinds: Vec<String>,
    },
    Extend {
        resume_at: DateTime<Utc>,
        kinds: Vec<String>,
    },
    Resume,
}

fn hot_limits<'a>(
    limits: &'a [Limit],
    now: DateTime<Utc>,
    threshold: f64,
) -> impl Iterator<Item = &'a Limit> {
    limits
        .iter()
        .filter(move |limit| limit.resets_at > now && limit.percent_used >= threshold)
}

pub fn evaluate(
    limits: &[Limit],
    state: &State,
    now: DateTime<Utc>,
    threshold: f64,
    margin: Duration,
) -> Action {
    let hot: Vec<&Limit> = hot_limits(limits, now, threshold).collect();
    let latest_reset = hot.iter().map(|limit| limit.resets_at + margin).max();
    let kinds = || hot.iter().map(|limit| limit.kind.clone()).collect::<Vec<_>>();

    match (&state.wrap, latest_reset) {
        (Some(wrap), Some(resume_at)) if resume_at > wrap.resume_at => Action::Extend {
            resume_at,
            kinds: kinds(),
        },
        (Some(wrap), _) if now >= wrap.resume_at => Action::Resume,
        (Some(_), _) => Action::Nothing,
        (None, Some(resume_at)) => Action::WrapUp {
            resume_at,
            kinds: kinds(),
        },
        (None, None) => Action::Nothing,
    }
}

pub fn kind_label(kind: &str) -> &str {
    match kind {
        "five_hour" => "de 5 h",
        "seven_day" => "semanal",
        other => other,
    }
}

pub fn wrap_up_prompt(limits: &[Limit], kinds: &[String]) -> String {
    let detail = kinds
        .iter()
        .filter_map(|kind| limits.iter().find(|limit| &limit.kind == kind))
        .map(|limit| format!("{} al {:.0} %", kind_label(&limit.kind), limit.percent_used))
        .collect::<Vec<_>>()
        .join(" y ");
    format!(
        "[herdr-usage] El uso del plan va {detail}. Antes del corte, cierra lo que llevas: haz commit y push de tu rama y deja una nota breve de dónde te quedaste y qué sigue. No empieces nada nuevo; cuando se reinicie el uso te aviso para que retomes."
    )
}

pub const RESUME_PROMPT: &str = "[herdr-usage] Ya se reinició el uso del plan. Retoma donde te quedaste, según la nota que dejaste antes del corte.";

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 8, hour, 0, 0).unwrap()
    }

    fn limit(kind: &str, percent: f64, reset_hour: u32) -> Limit {
        Limit {
            kind: kind.into(),
            percent_used: percent,
            resets_at: at(reset_hour),
        }
    }

    const MARGIN: i64 = 2;

    fn run(limits: &[Limit], state: &State, now_hour: u32) -> Action {
        evaluate(limits, state, at(now_hour), 90.0, Duration::minutes(MARGIN))
    }

    #[test]
    fn parses_meter_file() {
        let raw = r#"{"limits":[{"kind":"five_hour","percentUsed":4,"resetsAt":"2026-11-08T10:10:00.000Z"}]}"#;
        let limits = parse_meter(raw).unwrap();
        assert_eq!(limits[0].kind, "five_hour");
        assert_eq!(limits[0].percent_used, 4.0);
    }

    #[test]
    fn below_threshold_does_nothing() {
        let limits = [limit("five_hour", 50.0, 10)];
        assert_eq!(run(&limits, &State::default(), 8), Action::Nothing);
    }

    #[test]
    fn crossing_threshold_wraps_up_once() {
        let limits = [limit("five_hour", 91.0, 10)];
        let action = run(&limits, &State::default(), 8);
        let resume_at = at(10) + Duration::minutes(MARGIN);
        assert_eq!(
            action,
            Action::WrapUp {
                resume_at,
                kinds: vec!["five_hour".into()]
            }
        );
        let state = State {
            wrap: Some(Wrap {
                resume_at,
                kinds: vec!["five_hour".into()],
                panes: vec![],
                attempts: 0,
            }),
            timer_for: None,
        };
        assert_eq!(run(&limits, &state, 9), Action::Nothing);
    }

    #[test]
    fn stale_reading_after_reset_is_ignored() {
        let limits = [limit("five_hour", 99.0, 10)];
        assert_eq!(run(&limits, &State::default(), 11), Action::Nothing);
    }

    #[test]
    fn resumes_after_reset() {
        let limits = [limit("five_hour", 99.0, 10)];
        let state = State {
            wrap: Some(Wrap {
                resume_at: at(10),
                kinds: vec!["five_hour".into()],
                panes: vec![PaneRef {
                    pane_id: "w1:p1".into(),
                    session: None,
                }],
                attempts: 0,
            }),
            timer_for: None,
        };
        assert_eq!(run(&limits, &state, 11), Action::Resume);
    }

    #[test]
    fn weekly_limit_extends_resume_time() {
        let limits = [limit("five_hour", 95.0, 10), limit("seven_day", 92.0, 20)];
        let state = State {
            wrap: Some(Wrap {
                resume_at: at(10),
                kinds: vec!["five_hour".into()],
                panes: vec![],
                attempts: 0,
            }),
            timer_for: None,
        };
        assert_eq!(
            run(&limits, &state, 9),
            Action::Extend {
                resume_at: at(20) + Duration::minutes(MARGIN),
                kinds: vec!["five_hour".into(), "seven_day".into()]
            }
        );
    }

    fn agent(pane: &str, kind: &str, status: &str) -> Agent {
        Agent {
            pane_id: pane.into(),
            agent: kind.into(),
            status: status.into(),
            session: Some(format!("s-{pane}")),
        }
    }

    #[test]
    fn targets_working_claude_and_the_pane_that_just_finished() {
        let agents = [
            agent("w1:p1", "claude", "working"),
            agent("w1:p2", "claude", "idle"),
            agent("w1:p3", "claude", "idle"),
            agent("w1:p4", "codex", "working"),
            agent("w1:p5", "claude", "blocked"),
        ];
        let targets: Vec<String> = wrap_up_targets(&agents, Some("w1:p2"))
            .into_iter()
            .map(|pane| pane.pane_id)
            .collect();
        assert_eq!(targets, ["w1:p1", "w1:p2"]);
    }

    #[test]
    fn timer_is_armed_once_per_resume_time() {
        let wrap = Wrap {
            resume_at: at(10),
            kinds: vec!["five_hour".into()],
            panes: vec![],
            attempts: 0,
        };
        let mut state = State {
            wrap: Some(wrap),
            timer_for: None,
        };
        assert_eq!(timer_needed(&state), Some(at(10)));
        state.timer_for = Some(at(10));
        assert_eq!(timer_needed(&state), None);
        state.wrap.as_mut().unwrap().resume_at = at(20);
        assert_eq!(timer_needed(&state), Some(at(20)));
        assert_eq!(timer_needed(&State::default()), None);
    }

    #[test]
    fn session_name_comes_from_the_socket_path() {
        assert_eq!(session_from_socket(Some("/root/.config/herdr/herdr.sock")), "default");
        assert_eq!(
            session_from_socket(Some("/root/.config/herdr/sessions/blog/herdr.sock")),
            "blog"
        );
        assert_eq!(
            session_from_socket(Some("/root/.config/herdr/sessions/herdr/herdr.sock")),
            "herdr"
        );
        assert_eq!(session_from_socket(Some("/x/sessions/a b/herdr.sock")), "default");
        assert_eq!(session_from_socket(None), "default");
    }

    #[test]
    fn reads_rate_limits_from_statusline_input() {
        let raw = r#"{"model":{"id":"x"},"rate_limits":{"five_hour":{"used_percentage":46.2,"resets_at":1794258000},"seven_day":{"used_percentage":3,"resets_at":1794823200}}}"#;
        let limits = limits_from_statusline(raw).unwrap();
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].kind, "five_hour");
        assert_eq!(limits[0].percent_used, 46.2);
        assert_eq!(limits[0].resets_at.timestamp(), 1794258000);
        assert!(limits_from_statusline(r#"{"model":{}}"#).unwrap().is_empty());
        assert!(limits_from_statusline(r#"{"rate_limits":null}"#).unwrap().is_empty());
    }

    #[test]
    fn merge_ignores_stale_sessions_and_keeps_missing_windows() {
        let stored = [limit("five_hour", 60.0, 10), limit("seven_day", 20.0, 20)];
        let older_window = [limit("five_hour", 99.0, 5)];
        assert_eq!(merge_limits(&stored, &older_window), stored);
        let same_window_lower = [limit("five_hour", 40.0, 10)];
        assert_eq!(merge_limits(&stored, &same_window_lower)[0].percent_used, 60.0);
        let new_window = [limit("five_hour", 2.0, 15)];
        let merged = merge_limits(&stored, &new_window);
        assert_eq!(merged[0], limit("five_hour", 2.0, 15));
        assert_eq!(merged[1], limit("seven_day", 20.0, 20));
    }

    #[test]
    fn meter_round_trips_through_the_file_format() {
        let limits = vec![limit("five_hour", 46.0, 10)];
        assert_eq!(parse_meter(&meter_json(&limits)).unwrap(), limits);
    }

    #[test]
    fn renders_live_windows_only() {
        let limits = [limit("five_hour", 50.0, 10), limit("seven_day", 95.0, 7)];
        let line = render_meter(&limits, at(8), None);
        assert!(line.starts_with("5 h "));
        assert!(line.contains("▰▰▰▰\x1b[38;2;102;102;102m▱▱▱▱"));
        assert!(line.contains("50 %"));
        assert!(line.contains("(reinicia en 2 h 0 min)"));
        assert!(!line.contains("sem"));
        assert_eq!(render_meter(&[], at(8), None), "uso del plan: sin datos todavía");
    }

    fn visible_width(line: &str) -> usize {
        let mut width = 0;
        let mut in_escape = false;
        for c in line.chars() {
            match (in_escape, c) {
                (false, '\x1b') => in_escape = true,
                (true, 'm') => in_escape = false,
                (true, _) => {}
                (false, _) => width += 1,
            }
        }
        width
    }

    fn cells(line: &str) -> usize {
        line.chars().filter(|c| *c == '▰' || *c == '▱').count()
    }

    #[test]
    fn meter_fits_the_terminal_width() {
        let limits = [limit("five_hour", 50.0, 10), limit("seven_day", 20.0, 20)];
        for columns in [70, 80, 100, 120, 160] {
            let line = render_meter(&limits, at(8), Some(columns));
            assert!(visible_width(&line) <= columns - 10, "{columns}: {line}");
        }
        let wide = render_meter(&limits, at(8), Some(400));
        assert_eq!(cells(&wide), 60);
        assert!(wide.contains("reinicia en"));
        let medium = render_meter(&limits, at(8), Some(120));
        assert!(cells(&medium) > 16);
        assert!(medium.contains("reinicia en"));
        let narrow = render_meter(&limits, at(8), Some(70));
        assert!(!narrow.contains("reinicia en"));
        assert!(narrow.contains("(2 h 0 min)"));
        assert!(cells(&narrow) >= 12);
    }

    #[test]
    fn stable_link_follows_the_current_install() {
        let exe = Path::new("/plugins/herdr-usage/b/target/release/herdr-usage");
        let old = Path::new("/plugins/herdr-usage/a/target/release/herdr-usage");
        assert_eq!(stable_link_action(LinkState::Missing, exe), LinkAction::Create);
        assert_eq!(stable_link_action(LinkState::Symlink(exe), exe), LinkAction::Keep);
        assert_eq!(stable_link_action(LinkState::Symlink(old), exe), LinkAction::Replace);
        assert_eq!(stable_link_action(LinkState::OtherFile, exe), LinkAction::Blocked);
    }

    #[test]
    fn prompt_names_the_hot_limits() {
        let limits = [limit("five_hour", 91.4, 10)];
        let prompt = wrap_up_prompt(&limits, &["five_hour".into()]);
        assert!(prompt.contains("de 5 h al 91 %"));
    }

    fn wrap_with(panes: &[&str], attempts: u32) -> Wrap {
        Wrap {
            resume_at: at(10),
            kinds: vec!["five_hour".into()],
            panes: panes
                .iter()
                .map(|pane| PaneRef {
                    pane_id: (*pane).into(),
                    session: Some(format!("s-{pane}")),
                })
                .collect(),
            attempts,
        }
    }

    #[test]
    fn catch_up_reaches_working_panes_not_asked_yet() {
        let agents = [
            agent("w1:p1", "claude", "working"),
            agent("w1:p2", "claude", "working"),
            agent("w1:p3", "claude", "idle"),
        ];
        let targets: Vec<String> = catch_up_targets(&agents, None, &wrap_with(&["w1:p1"], 0))
            .into_iter()
            .map(|pane| pane.pane_id)
            .collect();
        assert_eq!(targets, ["w1:p2"]);
    }

    #[test]
    fn catch_up_only_runs_before_the_first_resume() {
        let state = |attempts| State {
            wrap: Some(wrap_with(&[], attempts)),
            timer_for: None,
        };
        assert!(in_wrap_window(&state(0), at(9)));
        assert!(!in_wrap_window(&state(0), at(10)));
        assert!(!in_wrap_window(&state(1), at(9)));
        assert!(!in_wrap_window(&State::default(), at(9)));
    }

    #[test]
    fn resume_needs_the_same_session_at_rest() {
        let pane = PaneRef {
            pane_id: "w1:p1".into(),
            session: Some("a".into()),
        };
        assert_eq!(readiness(&pane, Some("a"), "idle"), Readiness::Ready);
        assert_eq!(readiness(&pane, Some("a"), "done"), Readiness::Ready);
        assert_eq!(readiness(&pane, Some("a"), "working"), Readiness::Skip);
        assert_eq!(readiness(&pane, Some("b"), "idle"), Readiness::Skip);
    }

    #[test]
    fn failed_resume_retries_a_bounded_number_of_times() {
        let failed = || wrap_with(&["w1:p2"], 0).panes;
        let first = retry_resume(&wrap_with(&["w1:p1", "w1:p2"], 0), failed(), at(10)).unwrap();
        assert_eq!(first.resume_at, at(10) + Duration::minutes(RESUME_RETRY_MINUTES));
        assert_eq!(first.panes, failed());
        assert_eq!(first.attempts, 1);
        let second = retry_resume(&first, failed(), at(11)).unwrap();
        assert_eq!(second.attempts, 2);
        assert_eq!(retry_resume(&second, failed(), at(12)), None);
        assert_eq!(retry_resume(&first, vec![], at(11)), None);
    }

    #[test]
    fn old_state_files_without_attempts_still_load() {
        let raw = r#"{"wrap":{"resume_at":"2026-11-08T10:00:00Z","kinds":["five_hour"],"panes":[]}}"#;
        let state: State = serde_json::from_str(raw).unwrap();
        assert_eq!(state.wrap.unwrap().attempts, 0);
    }
}
