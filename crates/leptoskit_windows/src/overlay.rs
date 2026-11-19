//! The owned popup windows a `Modal` and a `Popover` show in.

use std::cell::Cell;
use std::rc::Rc;
use std::rc::Weak;
use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering;

use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::Graphics::Gdi::GetSysColorBrush;
use windows::Win32::Graphics::Gdi::PtInRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;
use windows::core::w;

use crate::backend::Widget;

const SHELL_CLASS: PCWSTR = w!("Leptoskit.NativeOverlay");

/// The shell class's atom, which tells a shell from any other top-level
/// window without asking for its class name.
static SHELL_ATOM: AtomicU16 = AtomicU16::new(0);

/// Registers the window class every overlay's shell uses.
pub(crate) fn register() -> windows::core::Result<()> {
	// SAFETY: The static class procedure tolerates a window with no context,
	// and the class outlives every window made from it.
	unsafe {
		let instance = GetModuleHandleW(None)?;
		let class = WNDCLASSW {
			lpfnWndProc: Some(shell_proc),
			hInstance: instance.into(),
			lpszClassName: SHELL_CLASS,
			hCursor: LoadCursorW(None, IDC_ARROW)?,
			hbrBackground: GetSysColorBrush(windows::Win32::Graphics::Gdi::COLOR_WINDOW),
			..Default::default()
		};
		let atom = RegisterClassW(&raw const class);

		if atom == 0 {
			if GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
				return Err(windows::core::Error::from_win32());
			}
		} else {
			SHELL_ATOM.store(atom, Ordering::Relaxed);
		}
	}

	Ok(())
}

/// The window a message loop treats as the dialog for a message: the
/// overlay shell the message's window is in, or else `host`. Tab and Escape
/// then work inside an overlay as they do in the host.
pub(crate) fn dialog_for(target: HWND, host: HWND) -> HWND {
	let atom = SHELL_ATOM.load(Ordering::Relaxed);

	if atom == 0 || target.is_invalid() {
		return host;
	}

	// SAFETY: These queries accept any window handle, live or not.
	unsafe {
		let root = GetAncestor(target, GA_ROOT);

		if !root.is_invalid() && GetClassLongPtrW(root, GCW_ATOM) as u16 == atom {
			root
		} else {
			host
		}
	}
}

/// What a shell's window procedure knows of its overlay.
struct Context {
	widget: Weak<Widget>,
	alive: Rc<Cell<bool>>,
}

/// Where an open overlay goes.
struct Target {
	/// The top-level window the overlay shows above.
	owner: HWND,
	/// The part of the viewport the shell covers, in layout units.
	rect: [f32; 4],
}

/// The popup window of one `Modal` or `Popover`. The widget's own group
/// window is a child of it while the overlay is open, and parked otherwise.
pub(crate) struct Shell {
	hwnd: Cell<HWND>,
	alive: Rc<Cell<bool>>,
	wanted: Cell<bool>,
	shown: Cell<bool>,
	/// The window a shown modal has disabled.
	owner: Cell<HWND>,
	/// What had focus when the overlay opened, which gets it back.
	before: Cell<HWND>,
}

impl Shell {
	pub(crate) fn new() -> Self {
		Self {
			hwnd: Cell::new(HWND::default()),
			alive: Rc::new(Cell::new(false)),
			wanted: Cell::new(false),
			shown: Cell::new(false),
			owner: Cell::new(HWND::default()),
			before: Cell::new(HWND::default()),
		}
	}

	/// The shell's window, once the overlay has been shown.
	pub(crate) fn window(&self) -> Option<HWND> {
		self.alive.get().then(|| self.hwnd.get())
	}

	pub(crate) fn is_shown(&self) -> bool {
		self.alive.get() && self.shown.get()
	}

	/// Where the overlay goes, or `None` while it is closed or the tree has
	/// no visible window to show it above.
	fn target(widget: &Widget) -> Option<Target> {
		let open = widget
			.props
			.borrow()
			.overlay
			.as_ref()
			.is_some_and(|overlay| overlay.open);
		let parent = widget.parent.borrow().upgrade()?;

		if !open || !widget.interactive() || !parent.frame_alive.get() {
			return None;
		}

		let host = widget.host.get();
		// SAFETY: These queries only read windows this thread owns.
		let owner = unsafe { GetAncestor(parent.frame, GA_ROOT) };

		// SAFETY: As above.
		if host == widget.parking
			|| owner.is_invalid()
			|| owner == widget.parking
			|| !unsafe { IsWindowVisible(owner) }.as_bool()
		{
			return None;
		}

		let [x, y, width, height] = widget.layout.get();
		let rect = if widget.kind == NativeKind::Modal {
			// A modal covers the viewport. Its window is what it holds.
			let held = widget
				.children
				.borrow()
				.iter()
				.filter(|child| child.shell.is_none() && child.visible())
				.map(|child| child.layout.get())
				.fold(None::<[f32; 4]>, |held, [left, top, width, height]| {
					let [right, bottom] = [left + width, top + height];
					Some(held.map_or([left, top, right, bottom], |held| {
						[
							held[0].min(left),
							held[1].min(top),
							held[2].max(right),
							held[3].max(bottom),
						]
					}))
				})?;
			[
				x + held[0],
				y + held[1],
				held[2] - held[0],
				held[3] - held[1],
			]
		} else {
			[x, y, width, height]
		};

		(rect[2] >= 1.0 && rect[3] >= 1.0).then_some(Target { owner, rect })
	}

	/// Sizes and places the shell for an open overlay, making it first if
	/// need be. Returns the shell and where in it the widget's own frame
	/// goes, in layout units, or `None` when the overlay is to stay down.
	pub(crate) fn arrange(&self, widget: &Rc<Widget>) -> Option<(HWND, f32, f32)> {
		let Some(target) = Self::target(widget) else {
			self.wanted.set(false);
			return None;
		};
		let scale = widget.scale();
		let hwnd = self.ensure(widget, target.owner)?;
		let mut corner = POINT {
			x: (target.rect[0] * scale).round() as i32,
			y: (target.rect[1] * scale).round() as i32,
		};
		// SAFETY: The host and the shell are live windows of this thread,
		// and the point is a local the call fills in.
		unsafe {
			// Layout measures from the corner of the host's client area.
			let _ = ClientToScreen(widget.host.get(), &raw mut corner);
			SetWindowPos(
				hwnd,
				None,
				corner.x,
				corner.y,
				(target.rect[2] * scale).round() as i32,
				(target.rect[3] * scale).round() as i32,
				SWP_NOZORDER | SWP_NOACTIVATE,
			)
			.expect("owned overlay shell accepts its bounds");
		}
		self.wanted.set(true);
		let [x, y, _, _] = widget.layout.get();
		Some((hwnd, x - target.rect[0], y - target.rect[1]))
	}

	/// The shell's window, owned by `owner`.
	fn ensure(&self, widget: &Rc<Widget>, owner: HWND) -> Option<HWND> {
		if self.alive.get() {
			// SAFETY: The shell is a live window of this thread.
			unsafe {
				if GetWindow(self.hwnd.get(), GW_OWNER).ok() == Some(owner) {
					return Some(self.hwnd.get());
				}
			}

			// A shell keeps the owner it was made with: make another.
			self.close();
		}

		let context = Rc::into_raw(Rc::new(Context {
			widget: Rc::downgrade(widget),
			alive: Rc::clone(&self.alive),
		}));
		// SAFETY: The shell is created and owned on this UI thread. Its
		// procedure takes the context from the creation parameters and
		// releases it on WM_NCDESTROY.
		let created = unsafe {
			CreateWindowExW(
				WS_EX_CONTROLPARENT | WS_EX_TOOLWINDOW,
				SHELL_CLASS,
				w!(""),
				WS_POPUP | WS_CLIPCHILDREN,
				0,
				0,
				0,
				0,
				Some(owner),
				None,
				GetModuleHandleW(None).ok().map(Into::into),
				Some(context.cast()),
			)
		};

		let Ok(hwnd) = created else {
			// SAFETY: No window took the context, so this is its only owner.
			unsafe {
				drop(Rc::from_raw(context));
			}
			return None;
		};
		self.hwnd.set(hwnd);
		self.alive.set(true);
		self.shown.set(false);
		Some(hwnd)
	}

	/// Shows the shell of an open overlay and hides that of a closed one,
	/// once the windows inside are in place. Returns whether this call
	/// showed it.
	pub(crate) fn present(&self, widget: &Widget) -> bool {
		if !self.alive.get() {
			return false;
		}

		if !self.wanted.get() {
			self.hide();
			return false;
		}

		if self.shown.replace(true) {
			return false;
		}

		let hwnd = self.hwnd.get();
		// SAFETY: The shell and its owner are live windows of this thread.
		unsafe {
			self.before.set(GetFocus());

			if widget.kind == NativeKind::Modal {
				let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
				self.owner.set(owner);
				// The window behind takes no input until the modal closes.
				let _ = EnableWindow(owner, false);
			}

			let _ = ShowWindow(hwnd, SW_SHOW);
		}
		true
	}

	/// Moves focus to `control`, or else to the first control in the shell's
	/// Tab order.
	pub(crate) fn focus(&self, control: Option<HWND>) {
		if !self.is_shown() {
			return;
		}

		// SAFETY: The shell is live, and the dialog manager only walks its
		// own child windows.
		unsafe {
			let target = control
				.filter(|control| IsWindowEnabled(*control).as_bool())
				.or_else(|| GetNextDlgTabItem(self.hwnd.get(), None, false).ok())
				.filter(|target| !target.is_invalid());

			if let Some(target) = target {
				let _ = SetFocus(Some(target));
			}
		}
	}

	/// Takes the shell off screen without asking anyone.
	pub(crate) fn hide(&self) {
		if !self.alive.get() || !self.shown.replace(false) {
			return;
		}

		// SAFETY: Every handle is checked before use, and belongs to this
		// thread.
		unsafe {
			let owner = self.owner.replace(HWND::default());

			// Enable the owner first, so that it and not some other
			// application becomes active when the shell goes.
			if !owner.is_invalid() && IsWindow(Some(owner)).as_bool() {
				let _ = EnableWindow(owner, true);
			}

			let inside = IsChild(self.hwnd.get(), GetFocus()).as_bool();
			let _ = ShowWindow(self.hwnd.get(), SW_HIDE);
			let before = self.before.replace(HWND::default());

			if inside && !before.is_invalid() && IsWindow(Some(before)).as_bool() {
				let _ = SetFocus(Some(before));
			}
		}
	}

	/// Takes the shell down for good. The widget's own windows must already
	/// be out of it, or gone.
	pub(crate) fn close(&self) {
		self.hide();

		if self.alive.get() {
			// SAFETY: The shell is a live window this thread owns.
			unsafe {
				let _ = DestroyWindow(self.hwnd.get());
			}
		}
	}
}

/// Whether `window` is `owner` or a popup some chain of owners leads from
/// to it: an overlay opened from inside another.
fn owned_by(mut window: HWND, owner: HWND) -> bool {
	// SAFETY: The owner query accepts any window handle.
	unsafe {
		while !window.is_invalid() {
			if window == owner {
				return true;
			}

			window = GetWindow(window, GW_OWNER).unwrap_or_default();
		}
	}

	false
}

/// Whether the primary button is down over the control a popover hangs
/// from. That press is the control's to answer: it decides whether its
/// popover stays.
fn pressing_anchor(widget: &Widget) -> bool {
	let Some(name) = widget
		.props
		.borrow()
		.overlay
		.as_ref()
		.and_then(|overlay| overlay.anchor.clone())
	else {
		return false;
	};
	let Some(anchor) = widget.named(&name) else {
		return false;
	};
	let mut cursor = POINT::default();
	let mut bounds = RECT::default();

	// SAFETY: Both calls fill locals, and the anchor's frame is checked
	// live.
	unsafe {
		GetKeyState(i32::from(VK_LBUTTON.0)) < 0
			&& anchor.frame_alive.get()
			&& GetCursorPos(&raw mut cursor).is_ok()
			&& GetWindowRect(anchor.frame, &raw mut bounds).is_ok()
			&& PtInRect(&raw const bounds, cursor).as_bool()
	}
}

unsafe extern "system" fn shell_proc(
	hwnd: HWND,
	message: u32,
	wparam: WPARAM,
	lparam: LPARAM,
) -> LRESULT {
	if message == WM_NCCREATE {
		// SAFETY: WM_NCCREATE carries the CREATESTRUCT whose parameter
		// `Shell::ensure` set to a retained context.
		unsafe {
			let create = &*(lparam.0 as *const CREATESTRUCTW);
			SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
		}
	}

	// SAFETY: Only WM_NCCREATE installs this pointer, which holds one Rc
	// reference until WM_NCDESTROY.
	let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Context;

	if pointer.is_null() {
		// SAFETY: Unhandled messages pass unchanged to the default procedure.
		return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
	}

	if message == WM_NCDESTROY {
		// SAFETY: Clear and release the installed context exactly once.
		unsafe {
			(*pointer).alive.set(false);
			SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
			drop(Rc::from_raw(pointer));

			return DefWindowProcW(hwnd, message, wparam, lparam);
		}
	}

	// SAFETY: The context is live until WM_NCDESTROY. The widget is kept
	// across a listener that may delete it and destroy this window.
	let widget = unsafe { (*pointer).widget.upgrade() };
	let ask = |widget: &Rc<Widget>| widget.sink.emit(widget, NativeEvent::Dismiss);
	let low = (wparam.0 & 0xffff) as u32;

	match (message, widget) {
		// Alt+F4. The overlay stays until the application closes it.
		(WM_CLOSE, Some(widget)) => {
			ask(&widget);
			LRESULT(0)
		}
		// The dialog manager turns Escape into the cancel command.
		(WM_COMMAND, Some(widget)) if low == IDCANCEL.0 as u32 => {
			ask(&widget);
			LRESULT(0)
		}
		(WM_ACTIVATE, Some(widget)) if low == WA_INACTIVE && widget.kind == NativeKind::Popover => {
			// Activation that moves to an overlay opened from inside this
			// one has not left it.
			if !owned_by(HWND(lparam.0 as *mut _), hwnd) && !pressing_anchor(&widget) {
				ask(&widget);
			}

			LRESULT(0)
		}
		// SAFETY: Unhandled messages pass unchanged to the default procedure.
		_ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
	}
}
