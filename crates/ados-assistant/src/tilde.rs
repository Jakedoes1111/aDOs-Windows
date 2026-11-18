//! Literal tildes in agent replies.
//!
//! gpui-kit parses Markdown with `ParseOptions::gfm()`, where GFM strikes through text
//! between single tildes (`~a~`), and it has no setting to turn that off. Agent replies
//! are full of shell prompts and home paths (`root@vm:~# cd ~/src`), so every lone `~`
//! outside code is escaped before rendering. `~~real strike~~` keeps working.

/// `text` with each lone `~` escaped as `\~`, except inside code fences, inline code,
/// `<autolinks>` and bare URLs, where an escape would show its backslash.
pub fn literal_tildes(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut fence: Option<(char, usize)> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() < 4 {
            if let Some((mark, len, rest)) = fence_of(trimmed) {
                match fence {
                    None => fence = Some((mark, len)),
                    Some((open, open_len)) if open == mark && len >= open_len && rest.trim().is_empty() => fence = None,
                    _ => {}
                }
                out.push_str(line);
                continue;
            }
        }
        if fence.is_some() {
            out.push_str(line);
        } else {
            escape_line(line, &mut out);
        }
    }
    out
}

/// A code fence opener or closer: its mark, run length and what follows the run.
fn fence_of(line: &str) -> Option<(char, usize, &str)> {
    let mark = line.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = line.chars().take_while(|c| *c == mark).count();
    (len >= 3).then(|| (mark, len, &line[len..]))
}

fn escape_line(line: &str, out: &mut String) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let run = chars[i..].iter().take_while(|x| **x == c).count();
        match c {
            '\\' => {
                // An existing escape stays as written, with the character it escapes.
                let end = (i + 2).min(chars.len());
                out.extend(&chars[i..end]);
                i = end;
            }
            '`' => {
                // Inline code: copy through the closing run of the same length, if any.
                let end = closing_backticks(&chars, i + run, run).unwrap_or(i + run);
                out.extend(&chars[i..end]);
                i = end;
            }
            '<' => match chars[i + 1..].iter().position(|x| *x == '>' || x.is_whitespace()) {
                Some(offset) if chars[i + 1 + offset] == '>' && chars[i + 1..i + 1 + offset].iter().any(|x| *x == ':' || *x == '@') => {
                    let end = i + offset + 2;
                    out.extend(&chars[i..end]);
                    i = end;
                }
                _ => {
                    out.push(c);
                    i += 1;
                }
            },
            '~' => {
                if run == 1 {
                    out.push_str("\\~");
                } else {
                    out.extend(&chars[i..i + run]);
                }
                i += run;
            }
            _ if starts_url(&chars[i..]) && (i == 0 || !chars[i - 1].is_alphanumeric()) => {
                let len = chars[i..].iter().take_while(|x| !x.is_whitespace()).count();
                out.extend(&chars[i..i + len]);
                i += len;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
}

fn closing_backticks(chars: &[char], from: usize, len: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        let run = chars[i..].iter().take_while(|x| **x == '`').count();
        if run == len {
            return Some(i + run);
        }
        i += run.max(1);
    }
    None
}

fn starts_url(chars: &[char]) -> bool {
    ["https://", "http://", "www."].iter().any(|prefix| {
        prefix.chars().count() <= chars.len() && prefix.chars().zip(chars).all(|(a, b)| a == *b)
    })
}

#[cfg(test)]
mod tests {
    use super::literal_tildes;

    #[test]
    fn shell_prompts_and_paths_keep_their_tildes() {
        assert_eq!(literal_tildes("root@vm:~# cd ~/src"), "root@vm:\\~# cd \\~/src");
    }

    #[test]
    fn double_tilde_strike_is_kept() {
        assert_eq!(literal_tildes("a ~~gone~~ b"), "a ~~gone~~ b");
    }

    #[test]
    fn code_is_untouched() {
        assert_eq!(literal_tildes("run `cd ~` now"), "run `cd ~` now");
        let fenced = "```sh\ncd ~/a\n~/b\n```\n~ x";
        assert_eq!(literal_tildes(fenced), "```sh\ncd ~/a\n~/b\n```\n\\~ x");
        let tilde_fence = "~~~\nroot@vm:~#\n~~~\n";
        assert_eq!(literal_tildes(tilde_fence), tilde_fence);
    }

    #[test]
    fn urls_and_escapes_are_untouched() {
        assert_eq!(literal_tildes("see https://x.org/~bob and <https://y.org/~al>"), "see https://x.org/~bob and <https://y.org/~al>");
        assert_eq!(literal_tildes("already \\~ escaped"), "already \\~ escaped");
    }

    #[test]
    fn unmatched_backtick_does_not_hide_tildes() {
        assert_eq!(literal_tildes("a ` b ~c"), "a ` b \\~c");
    }
}
