use crate::Router;

#[linkme::distributed_slice]
pub static PONS_TAKO_ROUTES: [fn(&mut Router)] = [..];

impl Router {
    pub fn mount_all(&mut self) -> &mut Self {
        self.mount_all_into("")
    }

    pub fn mount_all_into(&mut self, prefix: &str) -> &mut Self {
        let next = crate::router::join_prefix(&self.prefix, prefix);
        let saved = std::mem::replace(&mut self.prefix, next);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for register in PONS_TAKO_ROUTES {
                register(self);
            }
        }));
        self.prefix = saved;
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
        self
    }
}
