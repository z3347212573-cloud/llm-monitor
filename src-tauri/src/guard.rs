//! Consecutive-repetition detector for streaming model output.
//!
//! Port of the forensic scanner used on the real degeneration incidents
//! (glm-5.3-flash ". zai" x6047; deepseek-v4-flash "写 App." x6400):
//! segments the recent output tail, then looks for the maximal run of a
//! repeating short unit with period 1..=4. Pure-punctuation divider units
//! (`───`, `---`, `===`) are whitelisted — they are legitimate markdown.

const MIN_RUN: usize = 12;
const MAX_UNIT_CHARS: usize = 48;

fn is_divider_unit(unit: &str) -> bool {
    !unit.is_empty()
        && unit
            .chars()
            .all(|c| matches!(c, '─' | '-' | '–' | '=' | '—' | '_' | '.' | 'x' | 'X' | '*' | '#' | ' ' | '\t'))
}

/// Detect a consecutive repetition loop in the recent output `tail`.
/// Returns `(unit, total_run)` when a run of at least `MIN_RUN` repeats exists.
pub fn detect_loop(tail: &str) -> Option<(String, usize)> {
    // Split into delimiter-keeping segments (sentence-ish granularity).
    let mut raw: Vec<&str> = Vec::new();
    let mut start = 0usize;
    for (i, c) in tail.char_indices() {
        if matches!(c, '。' | '.' | '!' | '?' | '！' | '？' | '\n') {
            raw.push(&tail[start..i + c.len_utf8()]);
            start = i + c.len_utf8();
        }
    }
    raw.push(&tail[start..]);
    let segs: Vec<&str> = raw.iter().map(|s| s.trim()).collect();

    let mut best: Option<(String, usize)> = None;
    for period in 1..=4usize {
        let mut run = 0usize;
        for i in period..segs.len() {
            let cur = segs[i];
            let prev = segs[i - period];
            let ok = cur == prev
                && !cur.is_empty()
                && cur.chars().count() <= MAX_UNIT_CHARS
                && !is_divider_unit(cur);
            if ok {
                run += 1;
                let total = run + period;
                if total >= MIN_RUN && best.as_ref().is_none_or(|(_, r)| total > *r) {
                    best = Some((cur.to_string(), total));
                }
            } else {
                run = 0;
            }
        }
    }
    best
}

/// Shorten a detected unit for display in alerts.
pub fn shorten_unit(unit: &str) -> String {
    let trimmed = unit.trim();
    let mut out: String = trimmed.chars().take(24).collect();
    if trimmed.chars().count() > 24 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_simple_word_loop() {
        let tail = "前面正常的话。".to_string() + &"zai. ".repeat(20);
        let (unit, run) = detect_loop(&tail).expect("should detect");
        assert!(unit.contains("zai"));
        assert!(run >= 12);
    }

    #[test]
    fn detects_alternating_loop() {
        let tail = "开始。".to_string() + &"还在吗？运行成功：".repeat(20);
        assert!(detect_loop(&tail).is_some());
    }

    #[test]
    fn ignores_markdown_divider() {
        let tail = "标题\n─────\n内容\n─────\n标题2\n─────\n".repeat(30);
        assert!(detect_loop(&tail).is_none());
    }

    #[test]
    fn ignores_normal_text() {
        let tail = "这是一段完全正常的中文回复，包含标点和内容，没有任何连续重复的短语。模型在解释 TTFT 的含义。".repeat(3);
        assert!(detect_loop(&tail).is_none());
    }
}
