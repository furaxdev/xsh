pub fn matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    match_at(&p, 0, &t, 0)
}

fn match_at(p: &[char], pi: usize, t: &[char], ti: usize) -> bool {
    if pi == p.len() {
        return ti == t.len();
    }
    match p[pi] {
        '*' => {
            let mut j = ti;
            loop {
                if match_at(p, pi + 1, t, j) {
                    return true;
                }
                if j == t.len() {
                    return false;
                }
                j += 1;
            }
        }
        '?' => ti < t.len() && match_at(p, pi + 1, t, ti + 1),
        '[' => {
            if ti >= t.len() {
                return false;
            }
            let mut j = pi + 1;
            let negate = j < p.len() && (p[j] == '!' || p[j] == '^');
            if negate {
                j += 1;
            }
            let start = j;
            let mut found = false;
            while j < p.len() && (p[j] != ']' || j == start) {
                if j + 2 < p.len() && p[j + 1] == '-' && p[j + 2] != ']' {
                    if t[ti] >= p[j] && t[ti] <= p[j + 2] {
                        found = true;
                    }
                    j += 3;
                } else {
                    if p[j] == t[ti] {
                        found = true;
                    }
                    j += 1;
                }
            }
            if j >= p.len() {
                // unterminated bracket, treat '[' literally
                return p[pi] == t[ti] && match_at(p, pi + 1, t, ti + 1);
            }
            let matched = if negate { !found } else { found };
            matched && match_at(p, j + 1, t, ti + 1)
        }
        c => ti < t.len() && t[ti] == c && match_at(p, pi + 1, t, ti + 1),
    }
}
