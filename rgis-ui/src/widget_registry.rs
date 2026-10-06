use std::sync::Mutex;

#[cfg(target_arch = "wasm32")]
mod inner {
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static POSITIONS: RefCell<HashMap<String, [f32; 4]>> = RefCell::new(HashMap::new());
    }

    pub fn register(label: &str, rect: bevy_egui::egui::Rect) {
        POSITIONS.with(|p| {
            p.borrow_mut().insert(
                label.to_string(),
                [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
            );
        });
    }

    pub fn get(label: &str) -> Option<[f32; 4]> {
        POSITIONS.with(|p| p.borrow().get(label).copied())
    }

    pub fn get_all() -> HashMap<String, [f32; 4]> {
        POSITIONS.with(|p| p.borrow().clone())
    }
}

#[cfg(target_arch = "wasm32")]
pub use inner::*;

#[cfg(not(target_arch = "wasm32"))]
#[inline]
pub fn register(_label: &str, _rect: bevy_egui::egui::Rect) {}

/// Titles of windows that have been asked to close (by the wasm test hooks or
/// by `rgis-automation`). A `Mutex` rather than a thread-local because on
/// native the requester and the render systems run on different threads.
static CLOSE_REQUESTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn request_close(title: &str) {
    if let Ok(mut requests) = CLOSE_REQUESTS.lock() {
        requests.push(title.to_string());
    }
}

pub fn take_close_request(title: &str) -> bool {
    let Ok(mut requests) = CLOSE_REQUESTS.lock() else {
        return false;
    };
    if let Some(pos) = requests.iter().position(|t| t == title) {
        requests.remove(pos);
        true
    } else {
        false
    }
}
