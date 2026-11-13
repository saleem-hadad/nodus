//! Finding visuals and forms in Claude's Markdown: a fenced code block whose
//! info string is `visual` (or `vega-lite`, `vegalite`) holds a visual's
//! JSON, and one whose info string is `questions` holds a form's. Fences are
//! found as CommonMark finds them: ``` or ~~~, three or more, indented at
//! most three spaces, closed by a run of the same character at least as long.
//! Every other fence stays in the Markdown as it was.

/// A piece of a reply, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Markdown to show as it is, other fenced blocks included.
    Markdown(String),
    /// A visual's JSON, as written inside its fence.
    Visual(String),
    /// A form's JSON, as written inside its fence.
    Questions(String),
}

/// What a fenced block holds, from its info string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Visual,
    Questions,
    Other,
}

/// An opening fence: its character, its length and its indentation.
#[derive(Debug, Clone, Copy)]
struct Fence {
    ch: u8,
    len: usize,
    indent: usize,
    kind: Kind,
}

/// Splits a reply into Markdown, visuals and forms, in order.
///
/// A visual or questions fence that is never closed stays Markdown (Claude
/// may still be writing it). Markdown that is only whitespace between two
/// blocks is left out, so the list has no empty pieces.
pub fn split_fences(markdown: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    // Markdown not yet pushed: from `pending` to the current line.
    let mut pending = 0;
    // The fence we're inside, where it started (its first line's offset),
    // and where its content starts.
    let mut open: Option<(Fence, usize, usize)> = None;
    let mut at = 0;
    while at < markdown.len() {
        let end = markdown[at..].find('\n').map_or(markdown.len(), |i| at + i + 1);
        let line = markdown[at..end].trim_end_matches(['\n', '\r']);
        match open {
            None => {
                if let Some(fence) = opening(line) {
                    open = Some((fence, at, end));
                }
            }
            Some((fence, start, body)) => {
                if closes(line, fence) {
                    if fence.kind != Kind::Other {
                        push_markdown(&mut out, &markdown[pending..start]);
                        let content = strip_indent(&markdown[body..at], fence.indent);
                        out.push(match fence.kind {
                            Kind::Visual => Segment::Visual(content),
                            _ => Segment::Questions(content),
                        });
                        pending = end;
                    }
                    open = None;
                }
            }
        }
        at = end;
    }
    push_markdown(&mut out, &markdown[pending..]);
    out
}

fn push_markdown(out: &mut Vec<Segment>, text: &str) {
    if !text.trim().is_empty() {
        out.push(Segment::Markdown(text.to_owned()));
    }
}

/// The fence a line opens, if it opens one.
fn opening(line: &str) -> Option<Fence> {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ch = *rest.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = rest.bytes().take_while(|&b| b == ch).count();
    if len < 3 {
        return None;
    }
    let info = rest[len..].trim();
    // A backtick fence's info string can't hold a backtick: that's inline
    // code at the start of a line.
    if ch == b'`' && info.contains('`') {
        return None;
    }
    let word = info.split_whitespace().next().unwrap_or("");
    let kind = if ["visual", "vega-lite", "vegalite"]
        .iter()
        .any(|w| word.eq_ignore_ascii_case(w))
    {
        Kind::Visual
    } else if word.eq_ignore_ascii_case("questions") {
        Kind::Questions
    } else {
        Kind::Other
    };
    Some(Fence { ch, len, indent, kind })
}

/// Whether a line closes `fence`: at most three spaces, a run of its
/// character at least as long, then only spaces or tabs.
fn closes(line: &str, fence: Fence) -> bool {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let len = rest.bytes().take_while(|&b| b == fence.ch).count();
    len >= fence.len && rest[len..].bytes().all(|b| b == b' ' || b == b'\t')
}

/// The block's content with up to `indent` leading spaces taken off each
/// line, as CommonMark takes the opening fence's indentation off.
fn strip_indent(content: &str, indent: usize) -> String {
    let content = content.strip_suffix('\n').unwrap_or(content);
    let content = content.strip_suffix('\r').unwrap_or(content);
    if indent == 0 {
        return content.to_owned();
    }
    content
        .split('\n')
        .map(|line| {
            let spaces = line.bytes().take(indent).take_while(|&b| b == b' ').count();
            &line[spaces..]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(s: &str) -> Segment {
        Segment::Markdown(s.into())
    }
    fn vis(s: &str) -> Segment {
        Segment::Visual(s.into())
    }
    fn qs(s: &str) -> Segment {
        Segment::Questions(s.into())
    }

    #[test]
    fn plain_markdown_is_one_segment() {
        assert_eq!(split_fences("Hello\n\nworld"), vec![md("Hello\n\nworld")]);
        assert_eq!(split_fences(""), vec![]);
        assert_eq!(split_fences("  \n\n"), vec![]);
    }

    #[test]
    fn a_visual_fence_splits_the_reply() {
        let text = "Here it is:\n\n```visual\n{\"mark\": \"bar\"}\n```\n\nThat's all.";
        assert_eq!(
            split_fences(text),
            vec![md("Here it is:\n\n"), vis("{\"mark\": \"bar\"}"), md("\nThat's all.")]
        );
    }

    #[test]
    fn vega_lite_names_and_questions() {
        let text = "```vega-lite\n{}\n```\n```vegalite\n[]\n```\n~~~questions\n{\"title\": \"Q\"}\n~~~\n";
        assert_eq!(split_fences(text), vec![vis("{}"), vis("[]"), qs("{\"title\": \"Q\"}")]);
        // The info string's first word decides, in any case.
        assert_eq!(split_fences("```Visual extra words\n1\n```"), vec![vis("1")]);
    }

    #[test]
    fn other_fences_stay_in_the_markdown() {
        let text = "Run:\n```sh\ncargo test\n```\nthen\n```visual\n{}\n```";
        assert_eq!(
            split_fences(text),
            vec![md("Run:\n```sh\ncargo test\n```\nthen\n"), vis("{}")]
        );
        assert_eq!(split_fences("```\nplain\n```"), vec![md("```\nplain\n```")]);
    }

    #[test]
    fn a_visual_fence_inside_another_fence_is_left_alone() {
        let text = "````markdown\n```visual\n{}\n```\n````\n";
        assert_eq!(split_fences(text), vec![md(text)]);
        let tilde = "~~~\n```visual\n{}\n```\n~~~";
        assert_eq!(split_fences(tilde), vec![md(tilde)]);
    }

    #[test]
    fn a_longer_fence_needs_a_closer_as_long() {
        let text = "`````visual\n{\"a\": \"```\"}\n```\nmore\n`````\nafter";
        assert_eq!(
            split_fences(text),
            vec![vis("{\"a\": \"```\"}\n```\nmore"), md("after")]
        );
        // A closer may be longer than its opener.
        assert_eq!(split_fences("```visual\n{}\n``````"), vec![vis("{}")]);
        // A tilde doesn't close a backtick fence.
        assert_eq!(split_fences("```visual\n{}\n~~~\n```"), vec![vis("{}\n~~~")]);
    }

    #[test]
    fn indentation_up_to_three_spaces() {
        assert_eq!(
            split_fences("   ```visual\n   {\n     \"x\": 1\n   }\n   ```"),
            vec![vis("{\n  \"x\": 1\n}")]
        );
        // Only as much as the fence had is taken off.
        assert_eq!(split_fences("  ```visual\n    {}\n```"), vec![vis("  {}")]);
        // Four spaces is an indented code block, not a fence.
        let four = "    ```visual\n    {}\n    ```";
        assert_eq!(split_fences(four), vec![md(four)]);
        // A closer indented four spaces is content.
        assert_eq!(split_fences("```visual\n{}\n    ```\n```"), vec![vis("{}\n    ```")]);
    }

    #[test]
    fn an_unclosed_fence_stays_markdown() {
        let text = "Drawing it now:\n```visual\n{\"mark\": \"ba";
        assert_eq!(split_fences(text), vec![md(text)]);
        // A closed one before it is still found.
        let text = "```visual\n{}\n```\nand\n```questions\n{";
        assert_eq!(split_fences(text), vec![vis("{}"), md("and\n```questions\n{")]);
    }

    #[test]
    fn closers_take_only_trailing_spaces() {
        assert_eq!(split_fences("```visual\n{}\n```  \t\n"), vec![vis("{}")]);
        // Words after the run make it content.
        assert_eq!(split_fences("```visual\n{}\n``` no\n```"), vec![vis("{}\n``` no")]);
    }

    #[test]
    fn backticks_in_the_info_string_are_not_a_fence() {
        let text = "```visual``` is how you draw\n{}\n```";
        // The first line is inline code, so the last line opens a fence
        // that never closes.
        assert_eq!(split_fences(text), vec![md(text)]);
        // Tildes may have backticks after them.
        assert_eq!(split_fences("~~~visual `x`\n{}\n~~~"), vec![vis("{}")]);
    }

    #[test]
    fn crlf_lines_and_empty_blocks() {
        assert_eq!(
            split_fences("a\r\n```visual\r\n{}\r\n```\r\nb"),
            vec![md("a\r\n"), vis("{}"), md("b")]
        );
        assert_eq!(split_fences("```visual\n```"), vec![vis("")]);
        assert_eq!(split_fences("```questions\n\n```"), vec![qs("")]);
    }

    #[test]
    fn two_fences_back_to_back_have_nothing_between() {
        assert_eq!(
            split_fences("```visual\n1\n```\n\n```visual\n2\n```\n"),
            vec![vis("1"), vis("2")]
        );
    }
}
