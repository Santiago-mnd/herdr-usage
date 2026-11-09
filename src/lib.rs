use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

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

const BAR_CELLS: usize = 8;
const CLAUDE_ORANGE: &str = "\x1b[38;2;215;119;87m";
const AMBER: &str = "\x1b[38;2;255;193;7m";
const RED: &str = "\x1b[38;2;255;107;128m";
const GRAY: &str = "\x1b[38;2;102;102;102m";
const RESET: &str = "\x1b[0m";

pub fn render_meter(limits: &[Limit], now: DateTime<Utc>) -> String {
    let parts: Vec<String> = limits
        .iter()
        .filter(|limit| limit.resets_at > now)
        .map(|limit| {
            let percent = limit.percent_used.clamp(0.0, 100.0);
            let filled = ((percent / 100.0) * BAR_CELLS as f64).round() as usize;
            let color = match percent {
                p if p >= 90.0 => RED,
                p if p >= 70.0 => AMBER,
                _ => CLAUDE_ORANGE,
            };
            format!(
                "{} {}{}{}{}{} {:.0} %{RESET} ↺ {}",
                short_label(&limit.kind),
                color,
                "▰".repeat(filled),
                GRAY,
                "▱".repeat(BAR_CELLS - filled),
                color,
                percent,
                remaining_text(limit.resets_at - now)
            )
        })
        .collect();
    if parts.is_empty() {
        "uso del plan: sin datos todavía".into()
    } else {
        parts.join("  ·  ")
    }
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
        let line = render_meter(&limits, at(8));
        assert!(line.starts_with("5 h "));
        assert!(line.contains("▰▰▰▰\x1b[38;2;102;102;102m▱▱▱▱"));
        assert!(line.contains("50 %"));
        assert!(line.contains("↺ 2 h 0 min"));
        assert!(!line.contains("sem"));
        assert_eq!(render_meter(&[], at(8)), "uso del plan: sin datos todavía");
    }

    #[test]
    fn prompt_names_the_hot_limits() {
        let limits = [limit("five_hour", 91.4, 10)];
        let prompt = wrap_up_prompt(&limits, &["five_hour".into()]);
        assert!(prompt.contains("de 5 h al 91 %"));
    }
}
