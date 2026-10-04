//! Where leafcutter-ant reports what it did: a [`Console`] the host supplies,
//! with TS Dash's verbose timers on top (`src/Common/Console.ts`), and the
//! build [`Progress`] (`src/Core/Progress.ts`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

/// The host's console. The library never prints; everything it has to say
/// goes through these four methods, as it goes through TS Dash's `Console`.
pub trait Console {
	/// Ordinary progress messages.
	fn log(&self, message: &str);
	/// Information.
	fn info(&self, message: &str);
	/// Problems that do not stop anything, such as a pack folder that does
	/// not exist.
	fn warn(&self, message: &str);
	/// Failures, such as a plugin hook that threw.
	fn error(&self, message: &str);
}

/// The host's console plus TS Dash's `time` and `timeEnd`, which report only
/// when verbose logging is on.
pub(crate) struct Logger {
	console: Rc<dyn Console>,
	verbose: bool,
	timers: RefCell<HashMap<String, Instant>>,
}

impl Logger {
	pub(crate) fn new(console: Rc<dyn Console>, verbose: bool) -> Self {
		Logger {
			console,
			verbose,
			timers: RefCell::default(),
		}
	}

	pub(crate) fn console(&self) -> &dyn Console {
		&*self.console
	}

	pub(crate) fn time(&self, name: &str) {
		if !self.verbose {
			return;
		}
		let mut timers = self.timers.borrow_mut();
		if timers.contains_key(name) {
			self.console
				.warn(&format!("Timer \"{name}\" already exists."));
		} else {
			timers.insert(name.to_owned(), Instant::now());
		}
	}

	pub(crate) fn time_end(&self, name: &str) {
		if !self.verbose {
			return;
		}
		let started = self.timers.borrow_mut().remove(name);
		match started {
			Some(started) => self
				.console
				.log(&format!("{name}: {}ms", started.elapsed().as_millis())),
			None => self
				.console
				.warn(&format!("Timer \"{name}\" does not exist.")),
		}
	}
}

/// How far a build has come, for a progress bar: TS Dash's `Progress`.
pub struct Progress {
	total: Cell<u32>,
	current: Cell<u32>,
	listeners: RefCell<Vec<(ListenerId, Listener)>>,
	next_listener: Cell<u64>,
}

type Listener = Rc<dyn Fn(&Progress)>;

/// Returned by [`Progress::on_change`], to stop listening.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListenerId(u64);

impl Progress {
	pub(crate) fn new() -> Self {
		Progress {
			total: Cell::new(1),
			current: Cell::new(0),
			listeners: RefCell::default(),
			next_listener: Cell::new(0),
		}
	}

	/// `current / total`, as a fraction of 1. A build sets the total before
	/// it advances, and a total of zero gives a non-finite value, as in TS
	/// Dash.
	pub fn percentage(&self) -> f64 {
		f64::from(self.current.get()) / f64::from(self.total.get())
	}

	/// Calls `listener` after every change.
	pub fn on_change(&self, listener: impl Fn(&Progress) + 'static) -> ListenerId {
		let id = self.next_listener.get();
		self.next_listener.set(id + 1);
		self.listeners
			.borrow_mut()
			.push((ListenerId(id), Rc::new(listener)));
		ListenerId(id)
	}

	/// Stops calling the listener `id` was returned for.
	pub fn remove_listener(&self, id: ListenerId) {
		self.listeners
			.borrow_mut()
			.retain(|(listener, _)| *listener != id);
	}

	pub(crate) fn set_total(&self, total: u32) {
		self.total.set(total);
		self.current.set(0);
		self.changed();
	}

	/// `addToTotal(amount)`.
	pub(crate) fn add_to_total(&self, amount: u32) {
		self.total.set(self.total.get() + amount);
		self.changed();
	}

	pub(crate) fn advance(&self) {
		self.current.set(self.current.get() + 1);
		self.changed();
	}

	fn changed(&self) {
		// A listener may add or remove listeners, so call a copy of the list.
		let listeners: Vec<_> = self
			.listeners
			.borrow()
			.iter()
			.map(|(_, l)| Rc::clone(l))
			.collect();
		for listener in listeners {
			listener(self);
		}
	}
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;

	/// A console that keeps what it was told, as `level: message` lines.
	#[derive(Default)]
	pub(crate) struct Recorder(pub(crate) RefCell<Vec<String>>);

	impl Console for Recorder {
		fn log(&self, message: &str) {
			self.0.borrow_mut().push(format!("log: {message}"));
		}
		fn info(&self, message: &str) {
			self.0.borrow_mut().push(format!("info: {message}"));
		}
		fn warn(&self, message: &str) {
			self.0.borrow_mut().push(format!("warn: {message}"));
		}
		fn error(&self, message: &str) {
			self.0.borrow_mut().push(format!("error: {message}"));
		}
	}

	#[test]
	fn timers_report_only_when_verbose() {
		let recorder = Rc::new(Recorder::default());
		let quiet = Logger::new(recorder.clone(), false);
		quiet.time("a");
		quiet.time_end("a");
		quiet.time_end("never started");
		assert!(recorder.0.borrow().is_empty());

		let verbose = Logger::new(recorder.clone(), true);
		verbose.time("Loading files...");
		verbose.time("Loading files...");
		verbose.time_end("Loading files...");
		verbose.time_end("Loading files...");
		let lines = recorder.0.borrow();
		assert_eq!(lines[0], "warn: Timer \"Loading files...\" already exists.");
		assert!(
			lines[1].starts_with("log: Loading files...: ") && lines[1].ends_with("ms"),
			"{}",
			lines[1]
		);
		assert_eq!(lines[2], "warn: Timer \"Loading files...\" does not exist.");
		assert_eq!(lines.len(), 3);
	}

	#[test]
	fn progress_tells_every_listener_about_every_change() {
		let progress = Progress::new();
		let seen = Rc::new(RefCell::new(Vec::new()));
		let first = {
			let seen = seen.clone();
			progress.on_change(move |p| seen.borrow_mut().push(("first", p.percentage())))
		};
		{
			let seen = seen.clone();
			progress.on_change(move |p| seen.borrow_mut().push(("second", p.percentage())));
		}
		progress.set_total(4);
		progress.advance();
		progress.remove_listener(first);
		progress.advance();
		assert_eq!(
			*seen.borrow(),
			[
				("first", 0.0),
				("second", 0.0),
				("first", 0.25),
				("second", 0.25),
				("second", 0.5)
			]
		);
	}

	#[test]
	fn progress_starts_at_zero_of_one() {
		assert_eq!(Progress::new().percentage(), 0.0);
	}
}
