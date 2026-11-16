use std::collections::BTreeSet;

pub(super) struct Path {
    readable: String,
    escaped: String,
}

impl Path {
    pub(super) fn key(prefix: &str, key: &str) -> Self {
        Self {
            readable: format!("{prefix}.{key}"),
            escaped: format!("{prefix}.{}", escape(key)),
        }
    }

    pub(super) fn child(&self, segment: &str, key: &str) -> Self {
        Self {
            readable: format!("{}.{segment}.{key}", self.readable),
            escaped: format!("{}.{segment}.{}", self.escaped, escape(key)),
        }
    }

    pub(super) fn claim(&self, used: &mut BTreeSet<String>) -> String {
        if used.insert(self.readable.clone()) {
            return self.readable.clone();
        }
        let mut path = self.escaped.clone();
        let mut suffix = 2;
        while !used.insert(path.clone()) {
            path = format!("{}#{suffix}", self.readable);
            suffix += 1;
        }
        path
    }
}

fn escape(key: &str) -> String {
    crate::operation_path(&[vec![key.to_owned()]])
}
