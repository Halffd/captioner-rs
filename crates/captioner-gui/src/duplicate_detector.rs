use std::collections::HashSet;

pub struct DuplicateDetector {
    recent_lines: Vec<String>,
    threshold: f32,
    recent_count: usize,
}

impl DuplicateDetector {
    pub fn new(threshold: f32, recent_count: usize) -> Self {
        Self {
            recent_lines: Vec::new(),
            threshold,
            recent_count,
        }
    }

    pub fn add_line(&mut self, line: &str) {
        self.recent_lines.push(line.to_string());
        if self.recent_lines.len() > self.recent_count {
            self.recent_lines.remove(0);
        }
    }

    pub fn is_duplicate(&self, text: &str) -> bool {
        if self.recent_lines.is_empty() {
            return false;
        }

        let normalized = self.normalize(text);
        if normalized.is_empty() {
            return false;
        }

        for line in self.recent_lines.iter().rev().take(7) {
            let norm_line = self.normalize(line);
            if norm_line.is_empty() {
                continue;
            }
            if self.similarity(&normalized, &norm_line) > self.threshold {
                return true;
            }
        }

        false
    }

    fn normalize(&self, text: &str) -> String {
        text.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .take(50)
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn similarity(&self, s1: &str, s2: &str) -> f32 {
        let words1: HashSet<&str> = s1.split_whitespace().collect();
        let words2: HashSet<&str> = s2.split_whitespace().collect();

        if words1.is_empty() || words2.is_empty() {
            return 0.0;
        }

        let intersection = words1.intersection(&words2).count();
        let union = words1.union(&words2).count();

        if union == 0 {
            return 0.0;
        }

        intersection as f32 / union as f32
    }

    pub fn clear(&mut self) {
        self.recent_lines.clear();
    }
}

impl Default for DuplicateDetector {
    fn default() -> Self {
        Self::new(0.8, 20)
    }
}