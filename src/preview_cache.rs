//! A viewport-sized PDF render queue and a bounded cache of decoded pages.
//!
//! The compile/save worker never waits for this worker. Requests replace the
//! pending queue when the reader scrolls, while a page already being rendered
//! can finish and populate the PDF's disk cache.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::sync::{Arc, Condvar, Mutex, mpsc};

use rune::{Pixels, Waker};

use crate::preview::{self, Document};

const MEMORY_BUDGET: usize = 64 * 1024 * 1024;
const PREFETCH_PAGES: usize = 2;

struct CachedPage {
    pixels: Pixels,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Queue {
    document: Option<Arc<Document>>,
    version: u64,
    wanted: HashSet<usize>,
    pending: VecDeque<usize>,
    completed: HashSet<usize>,
    active: Option<(u64, usize)>,
    closed: bool,
}

impl Queue {
    fn accepts(&self, version: u64, index: usize) -> bool {
        !self.closed && self.version == version && self.wanted.contains(&index)
    }
}

struct Ready {
    version: u64,
    index: usize,
    result: Result<Pixels, String>,
}

pub struct PreviewCache {
    document: Option<Arc<Document>>,
    pages: HashMap<usize, CachedPage>,
    errors: HashMap<usize, String>,
    visible: Range<usize>,
    wanted: Vec<usize>,
    serviced: HashSet<usize>,
    version: u64,
    clock: u64,
    bytes: usize,
    budget: usize,
    queue: Arc<(Mutex<Queue>, Condvar)>,
    finished: mpsc::Receiver<Ready>,
}

impl PreviewCache {
    pub fn new(waker: Arc<Mutex<Option<Waker>>>) -> Self {
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        // A paused window must not accumulate a document's worth of completed
        // page images while its UI is not polling.
        let (sender, finished) = mpsc::sync_channel(2);
        let worker_queue = Arc::clone(&queue);
        std::thread::spawn(move || render_worker(worker_queue, sender, waker));
        Self {
            document: None,
            pages: HashMap::new(),
            errors: HashMap::new(),
            visible: 0..0,
            wanted: Vec::new(),
            serviced: HashSet::new(),
            version: 0,
            clock: 0,
            bytes: 0,
            budget: MEMORY_BUDGET,
            queue,
            finished,
        }
    }

    pub fn document(&self) -> Option<&Document> {
        self.document.as_deref()
    }

    /// Metadata alone never starts rasterization. The first actual viewport
    /// callback supplies the initial range, including for a restored PDF.
    pub fn set_document(&mut self, document: Option<Document>) {
        self.version = self.version.wrapping_add(1);
        self.document = document.map(Arc::new);
        self.pages.clear();
        self.errors.clear();
        self.bytes = 0;
        self.visible = 0..0;
        self.wanted.clear();
        self.serviced.clear();
        while self.finished.try_recv().is_ok() {}
        self.schedule();
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn page(&self, index: usize) -> Option<&Pixels> {
        self.pages.get(&index).map(|page| &page.pixels)
    }

    pub fn error(&self, index: usize) -> Option<&str> {
        self.errors.get(&index).map(String::as_str)
    }

    pub fn visible(&self) -> Range<usize> {
        self.visible.clone()
    }

    pub fn show_visible(&mut self, range: Range<usize>) -> bool {
        let count = self.document.as_ref().map_or(0, |pdf| pdf.pages.len());
        let range = range.start.min(count)..range.end.min(count);
        if self.visible == range {
            return false;
        }
        self.visible = range;
        self.wanted = wanted_pages(self.visible.clone(), count);
        self.errors.retain(|index, _| self.wanted.contains(index));
        self.serviced.retain(|index| self.wanted.contains(index));
        self.clock = self.clock.wrapping_add(1);
        for index in &self.wanted {
            if let Some(page) = self.pages.get_mut(index) {
                page.used = self.clock;
            }
        }
        self.evict();
        self.schedule();
        true
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(ready) = self.finished.try_recv() {
            if ready.version != self.version || !self.wanted.contains(&ready.index) {
                continue;
            }
            match ready.result {
                Ok(pixels) => self.store(ready.index, pixels),
                Err(error) => {
                    self.errors.insert(ready.index, error);
                }
            }
            self.serviced.insert(ready.index);
            self.queue
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .completed
                .remove(&ready.index);
            changed = true;
        }
        if changed {
            self.evict();
            self.schedule();
        }
        changed
    }

    fn store(&mut self, index: usize, pixels: Pixels) {
        let (width, height) = pixels.size();
        let bytes = width as usize * height as usize * 4;
        self.clock = self.clock.wrapping_add(1);
        if let Some(old) = self.pages.insert(
            index,
            CachedPage {
                pixels,
                bytes,
                used: self.clock,
            },
        ) {
            self.bytes -= old.bytes;
        }
        self.bytes += bytes;
        self.errors.remove(&index);
    }

    fn evict(&mut self) {
        // A very tall window may contain more than the budget. Keep its visible
        // pages intact; the budget bounds retained pages outside that window.
        while self.bytes > self.budget {
            let oldest = self
                .pages
                .iter()
                .filter(|(index, _)| !self.visible.contains(index))
                .min_by_key(|(index, page)| (page.used, **index))
                .map(|(index, _)| *index);
            let Some(index) = oldest else { break };
            if let Some(page) = self.pages.remove(&index) {
                self.bytes -= page.bytes;
            }
        }
    }

    fn schedule(&self) {
        let (lock, wake) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if queue.version != self.version {
            queue.completed.clear();
        }
        queue.version = self.version;
        queue.document = self.document.clone();
        queue.wanted = self.wanted.iter().copied().collect();
        let wanted = queue.wanted.clone();
        queue.completed.retain(|index| wanted.contains(index));
        queue.pending = self
            .wanted
            .iter()
            .copied()
            .filter(|index| {
                self.needs_page(*index)
                    && !queue.completed.contains(index)
                    && queue.active != Some((self.version, *index))
            })
            .collect();
        wake.notify_one();
    }

    fn needs_page(&self, index: usize) -> bool {
        !self.pages.contains_key(&index)
            && !self.errors.contains_key(&index)
            // An evicted prefetch must not immediately rerender itself when
            // the visible window already fills the memory budget. It becomes
            // eligible again when the reader actually reaches that page.
            && (!self.serviced.contains(&index) || self.visible.contains(&index))
    }
}

impl Drop for PreviewCache {
    fn drop(&mut self) {
        let (lock, wake) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.closed = true;
        queue.pending.clear();
        queue.document = None;
        wake.notify_one();
        // Do not join a renderer process from the UI thread. Dropping the
        // receiver also releases a worker blocked on the bounded send channel.
    }
}

fn wanted_pages(visible: Range<usize>, count: usize) -> Vec<usize> {
    if visible.is_empty() {
        return Vec::new();
    }
    let mut wanted: Vec<_> = visible.clone().collect();
    for distance in 1..=PREFETCH_PAGES {
        if let Some(before) = visible.start.checked_sub(distance) {
            wanted.push(before);
        }
        if let Some(after) = visible.end.checked_add(distance - 1).filter(|&i| i < count) {
            wanted.push(after);
        }
    }
    wanted
}

fn render_worker(
    queue: Arc<(Mutex<Queue>, Condvar)>,
    sender: mpsc::SyncSender<Ready>,
    waker: Arc<Mutex<Option<Waker>>>,
) {
    loop {
        let (document, version, index) = {
            let (lock, wake) = &*queue;
            let mut pending = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            while !pending.closed && pending.pending.is_empty() {
                pending = wake
                    .wait(pending)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            if pending.closed {
                return;
            }
            let Some(document) = pending.document.clone() else {
                pending.pending.clear();
                continue;
            };
            let index = pending.pending.pop_front().expect("queue is nonempty");
            let version = pending.version;
            pending.active = Some((version, index));
            (document, version, index)
        };
        let result = preview::render_page(&document, index);
        let accepted = {
            let mut pending = queue
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pending.active = None;
            if pending.accepts(version, index) {
                pending.completed.insert(index);
                true
            } else {
                false
            }
        };
        if !accepted {
            continue;
        }
        if sender
            .send(Ready {
                version,
                index,
                result,
            })
            .is_err()
        {
            return;
        }
        if let Some(wake) = waker.lock().ok().and_then(|slot| slot.clone()) {
            wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel() -> Pixels {
        let pixels = Pixels::new();
        pixels.set(1, 1, vec![255; 4]);
        pixels
    }

    #[test]
    fn visible_pages_precede_neighbors_and_thirty_is_not_special() {
        assert_eq!(wanted_pages(30..32, 100), vec![30, 31, 29, 32, 28, 33]);
        assert_eq!(wanted_pages(0..1, 1), vec![0]);
        assert_eq!(wanted_pages(98..100, 100), vec![98, 99, 97, 96]);
        assert!(wanted_pages(0..0, 100).is_empty());
    }

    #[test]
    fn late_results_cannot_replace_current_document_or_new_viewport() {
        let mut queue = Queue {
            version: 2,
            wanted: HashSet::from([40]),
            ..Queue::default()
        };
        assert!(!queue.accepts(1, 40));
        assert!(!queue.accepts(2, 0));
        assert!(queue.accepts(2, 40));
        queue.closed = true;
        assert!(!queue.accepts(2, 40));
    }

    #[test]
    fn lru_evicts_unseen_pages_and_pins_the_visible_window() {
        let mut cache = PreviewCache::new(Arc::new(Mutex::new(None)));
        cache.budget = 8;
        cache.store(0, pixel());
        cache.store(1, pixel());
        cache.store(2, pixel());
        cache.evict();
        assert!(cache.page(0).is_none());
        assert_eq!(cache.bytes, 8);
        cache.visible = 1..3;
        cache.budget = 4;
        cache.evict();
        assert!(cache.page(1).is_some());
        assert!(cache.page(2).is_some());
        assert_eq!(cache.bytes, 8);
        cache.visible = 2..3;
        cache.evict();
        assert!(cache.page(1).is_none());
        assert_eq!(cache.bytes, 4);
    }

    #[test]
    fn evicted_prefetch_waits_until_it_is_visible_instead_of_rerendering() {
        let mut cache = PreviewCache::new(Arc::new(Mutex::new(None)));
        cache.serviced.insert(3);
        cache.visible = 0..1;
        assert!(!cache.needs_page(3));
        cache.visible = 3..4;
        assert!(cache.needs_page(3));
    }
}
