//! The pure helpers of `store.ts` and `UsageStats.tsx`: durations, paths,
//! card titles, agent order and labels, token counts and costs
//! (UI-081…UI-087, UI-114, UI-115).

use std::collections::HashSet;

use nodus_domain::js;
use nodus_domain::{AgentInfo, SessionCard, SessionKind, SessionStatus};

/// `formatDuration`: a short span, as `42s`, `4m 12s` or `1h 5m` (UI-081).
pub fn format_duration(ms: f64) -> String {
    let s = js::round(ms / 1000.0).max(0.0) as u64;
    if s < 60 {
        return format!("{s}s");
    }
    let m = s / 60;
    if m < 60 {
        return if !s.is_multiple_of(60) && m < 10 {
            format!("{m}m {}s", s % 60)
        } else {
            format!("{m}m")
        };
    }
    format!("{}h {}m", m / 60, m % 60)
}

/// `tildePath`: the home folder abbreviated to `~`, as a shell prompt does
/// (UI-082). Only `/Users/<name>` and `/home/<name>` count.
pub fn tilde_path(path: &str) -> String {
    for root in ["/Users/", "/home/"] {
        if let Some(rest) = path.strip_prefix(root) {
            let name_end = rest.find('/').unwrap_or(rest.len());
            if name_end == 0 {
                return path.to_owned();
            }
            return format!("~{}", &rest[name_end..]);
        }
    }
    path.to_owned()
}

/// `cardTitle`: the user's title, else the transcript's, else a placeholder
/// (UI-083).
pub fn card_title(card: Option<&SessionCard>) -> String {
    let Some(card) = card else {
        return "New session".into();
    };
    if let Some(title) = card.node.title.as_deref().filter(|t| !t.is_empty()) {
        return title.to_owned();
    }
    if let Some(title) = card.meta.as_ref().map(|m| m.title.as_str()).filter(|t| !t.is_empty()) {
        return title.to_owned();
    }
    if card.node.kind == Some(SessionKind::Terminal) {
        "Terminal".into()
    } else {
        "New session".into()
    }
}

fn rank(status: SessionStatus) -> u8 {
    match status {
        SessionStatus::Waiting => 0,
        SessionStatus::Working => 1,
        SessionStatus::Done => 2,
    }
}

/// `byUrgency`: waiting agents first, then working, then done; ties by
/// start, earliest first (UI-084).
pub fn by_urgency(a: &AgentInfo, b: &AgentInfo) -> std::cmp::Ordering {
    rank(a.status).cmp(&rank(b.status)).then(
        a.started_at
            .partial_cmp(&b.started_at)
            .unwrap_or(std::cmp::Ordering::Equal),
    )
}

/// What an agent is doing, for the line under it (UI-085).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentStep {
    pub tool: Option<String>,
    pub text: Option<String>,
}

/// `agentStep`.
pub fn agent_step(agent: &AgentInfo) -> AgentStep {
    if agent.status == SessionStatus::Waiting {
        return AgentStep {
            tool: None,
            text: Some("Waiting for you in the session’s terminal".into()),
        };
    }
    if let Some(background) = agent.background.filter(|&n| n > 0) {
        let what = if background == 1 {
            "its background command".to_owned()
        } else {
            format!("{background} background commands")
        };
        return AgentStep {
            tool: None,
            text: Some(format!("Waiting for {what} to finish")),
        };
    }
    AgentStep {
        tool: agent.tool.clone(),
        text: agent.activity.clone(),
    }
}

/// `agentStatusLabel` (UI-086).
pub fn agent_status_label(agent: &AgentInfo) -> &'static str {
    match agent.status {
        SessionStatus::Waiting => "Needs you",
        SessionStatus::Working => "Working",
        SessionStatus::Done if agent.stopped == Some(true) => "Stopped",
        SessionStatus::Done => "Done",
    }
}

/// `nestAgents`: agents in the order they started (or newest first), each
/// followed by the agents it started, one level deeper. An agent whose parent
/// isn't in the list is top level (UI-087).
pub fn nest_agents(agents: &[AgentInfo], newest_first: bool) -> Vec<(AgentInfo, usize)> {
    let ids: HashSet<&str> = agents.iter().map(|a| a.agent_id.as_str()).collect();
    let mut sorted: Vec<&AgentInfo> = agents.iter().collect();
    // A stable sort, as Array.prototype.sort is.
    sorted.sort_by(|x, y| {
        let order = x
            .started_at
            .partial_cmp(&y.started_at)
            .unwrap_or(std::cmp::Ordering::Equal);
        if newest_first { order.reverse() } else { order }
    });
    let parent_of = |a: &AgentInfo| {
        a.parent_agent_id
            .as_deref()
            .filter(|p| !p.is_empty() && ids.contains(p))
            .map(str::to_owned)
    };
    let mut out = Vec::new();
    fn walk<'a>(
        parent: Option<&str>,
        depth: usize,
        sorted: &[&'a AgentInfo],
        parent_of: &dyn Fn(&AgentInfo) -> Option<String>,
        out: &mut Vec<(AgentInfo, usize)>,
    ) {
        for agent in sorted.iter().filter(|a| parent_of(a).as_deref() == parent) {
            out.push(((*agent).clone(), depth));
            walk(Some(&agent.agent_id), depth + 1, sorted, parent_of, out);
        }
    }
    walk(None, 0, &sorted, &parent_of, &mut out);
    out
}

/// `formatTokens`: 950, 9.5k, 106k, 1M, 1.5M (UI-115).
pub fn format_tokens(n: f64) -> String {
    if n < 1000.0 {
        return js_number(n);
    }
    if n < 1_000_000.0 {
        let k = if n < 10_000.0 {
            strip_point_zero(&to_fixed(n / 1000.0, 1))
        } else {
            js_number(js::round(n / 1000.0))
        };
        return format!("{k}k");
    }
    format!("{}M", strip_point_zero(&to_fixed(n / 1_000_000.0, 1)))
}

/// `formatCost`: `$1.23` under $100, `$123` from there (UI-114).
pub fn format_cost(usd: f64) -> String {
    if usd < 100.0 {
        format!("${}", to_fixed(usd, 2))
    } else {
        format!("${}", js_number(js::round(usd)))
    }
}

fn strip_point_zero(s: &str) -> String {
    s.strip_suffix(".0").unwrap_or(s).to_owned()
}

/// `String(n)`.
fn js_number(n: f64) -> String {
    nodus_domain::json::number_to_string(n)
}

/// `Number.prototype.toFixed(digits)`: the nearest number with that many
/// decimals, a tie going to the larger magnitude (Rust's formatter rounds a
/// tie to even).
pub fn to_fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() || x.abs() >= 1e21 {
        return js_number(x);
    }
    let negative = x < 0.0;
    // Enough digits past the cut to see the exact value: a double has at
    // most 17 significant ones, so a tie shows as 5 then zeros.
    let exact = format!("{:.*}", digits + 30, x.abs());
    let (int, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let keep = &frac[..digits];
    let rest = &frac[digits..];
    let mut number: Vec<u8> = int.bytes().chain(keep.bytes()).collect();
    if rest.as_bytes().first().is_some_and(|&d| d >= b'5') {
        // Carry the increment up.
        let mut i = number.len();
        loop {
            if i == 0 {
                number.insert(0, b'1');
                break;
            }
            i -= 1;
            if number[i] == b'9' {
                number[i] = b'0';
            } else {
                number[i] += 1;
                break;
            }
        }
    }
    let split = number.len() - digits;
    let int: String = number[..split].iter().map(|&b| b as char).collect();
    let frac: String = number[split..].iter().map(|&b| b as char).collect();
    let body = if digits == 0 { int } else { format!("{int}.{frac}") };
    if negative { format!("-{body}") } else { body }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{BoardNode, SessionMeta};

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(42_000.0), "42s");
        assert_eq!(format_duration(-5.0), "0s");
        assert_eq!(format_duration(252_000.0), "4m 12s");
        assert_eq!(format_duration(240_000.0), "4m");
        assert_eq!(format_duration(720_000.0 + 13_000.0), "12m");
        assert_eq!(format_duration(3_900_000.0), "1h 5m");
        assert_eq!(format_duration(59_500.0), "1m");
    }

    #[test]
    fn abbreviates_home_folders() {
        assert_eq!(tilde_path("/Users/example/Code/shop"), "~/Code/shop");
        assert_eq!(tilde_path("/home/example"), "~");
        assert_eq!(tilde_path("/Users/example"), "~");
        assert_eq!(tilde_path("/opt/repos"), "/opt/repos");
        assert_eq!(tilde_path("/Users//x"), "/Users//x");
    }

    #[test]
    fn titles_cards() {
        let node: BoardNode = serde_json::from_value(serde_json::json!({
            "sessionId": "s", "x": 0, "y": 0, "archived": false, "createdAt": 0
        }))
        .unwrap();
        let mut card = SessionCard {
            node,
            meta: None,
            status: SessionStatus::Done,
            live: false,
            agents: None,
            agent_count: None,
            usage: None,
        };
        assert_eq!(card_title(Some(&card)), "New session");
        card.node.kind = Some(SessionKind::Terminal);
        assert_eq!(card_title(Some(&card)), "Terminal");
        card.meta = Some(
            serde_json::from_value::<SessionMeta>(serde_json::json!({
                "sessionId": "s", "title": "Fix it", "messageCount": 0, "createdAt": 0,
                "updatedAt": 0, "transcriptPath": "/t"
            }))
            .unwrap(),
        );
        assert_eq!(card_title(Some(&card)), "Fix it");
        card.node.title = Some("Mine".into());
        assert_eq!(card_title(Some(&card)), "Mine");
    }

    fn agent(id: &str, status: SessionStatus, started: f64, parent: Option<&str>) -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "agentId": id, "agentType": "x", "description": "", "status": status,
            "toolCount": 0, "startedAt": started, "updatedAt": started,
            "parentAgentId": parent,
        }))
        .unwrap()
    }

    #[test]
    fn orders_agents_by_urgency() {
        let mut agents = [
            agent("a", SessionStatus::Done, 1.0, None),
            agent("b", SessionStatus::Working, 3.0, None),
            agent("c", SessionStatus::Waiting, 5.0, None),
            agent("d", SessionStatus::Working, 2.0, None),
        ];
        agents.sort_by(by_urgency);
        let ids: Vec<&str> = agents.iter().map(|a| a.agent_id.as_str()).collect();
        assert_eq!(ids, ["c", "d", "b", "a"]);
    }

    #[test]
    fn labels_and_steps_agents() {
        let mut a = agent("a", SessionStatus::Waiting, 0.0, None);
        assert_eq!(agent_status_label(&a), "Needs you");
        assert_eq!(
            agent_step(&a).text.as_deref(),
            Some("Waiting for you in the session’s terminal")
        );
        a.status = SessionStatus::Working;
        a.background = Some(1);
        assert_eq!(
            agent_step(&a).text.as_deref(),
            Some("Waiting for its background command to finish")
        );
        a.background = Some(3);
        assert_eq!(
            agent_step(&a).text.as_deref(),
            Some("Waiting for 3 background commands to finish")
        );
        a.background = None;
        a.tool = Some("Bash".into());
        a.activity = Some("pnpm test".into());
        assert_eq!(
            agent_step(&a),
            AgentStep {
                tool: Some("Bash".into()),
                text: Some("pnpm test".into())
            }
        );
        assert_eq!(agent_status_label(&a), "Working");
        a.status = SessionStatus::Done;
        assert_eq!(agent_status_label(&a), "Done");
        a.stopped = Some(true);
        assert_eq!(agent_status_label(&a), "Stopped");
    }

    #[test]
    fn nests_agents_under_the_agents_that_started_them() {
        let agents = [
            agent("child", SessionStatus::Working, 3.0, Some("root")),
            agent("root", SessionStatus::Working, 1.0, None),
            agent("orphan", SessionStatus::Working, 2.0, Some("gone")),
            agent("grandchild", SessionStatus::Working, 4.0, Some("child")),
        ];
        let nested: Vec<(String, usize)> = nest_agents(&agents, false)
            .into_iter()
            .map(|(a, d)| (a.agent_id, d))
            .collect();
        assert_eq!(
            nested,
            [
                ("root".into(), 0),
                ("child".into(), 1),
                ("grandchild".into(), 2),
                ("orphan".into(), 0)
            ]
        );
        let newest: Vec<String> = nest_agents(&agents, true)
            .into_iter()
            .map(|(a, _)| a.agent_id)
            .collect();
        assert_eq!(newest, ["orphan", "root", "child", "grandchild"]);
    }

    #[test]
    fn formats_tokens_and_costs() {
        assert_eq!(format_tokens(950.0), "950");
        assert_eq!(format_tokens(9_500.0), "9.5k");
        assert_eq!(format_tokens(9_000.0), "9k");
        assert_eq!(format_tokens(106_400.0), "106k");
        assert_eq!(format_tokens(1_000_000.0), "1M");
        assert_eq!(format_tokens(1_540_000.0), "1.5M");
        assert_eq!(format_cost(1.234), "$1.23");
        assert_eq!(format_cost(0.125), "$0.13");
        assert_eq!(format_cost(99.999), "$100.00");
        assert_eq!(format_cost(123.5), "$124");
    }

    #[test]
    fn fixes_decimals_as_javascript_does() {
        assert_eq!(to_fixed(0.125, 2), "0.13");
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(9.95, 1), "9.9");
        assert_eq!(to_fixed(0.0, 2), "0.00");
        assert_eq!(to_fixed(-1.5, 0), "-2");
        assert_eq!(to_fixed(-0.001, 2), "-0.00");
    }
}
