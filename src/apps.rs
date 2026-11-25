//! The apps dictations go to: which one is in front, and its icon, kept as
//! a small PNG for the Stats page to show.
#![allow(unexpected_cfgs)] // objc's `sel_impl!` checks a cfg this crate never sets

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;

use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

use crate::config::data_dir;

/// The app in front.
pub struct Front {
    /// As the Dock shows it.
    pub name: String,
    /// Its `.app` bundle.
    pub bundle: Option<String>,
}

pub fn frontmost() -> Option<Front> {
    unsafe {
        let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: *mut Object = msg_send![workspace, frontmostApplication];
        if app.is_null() {
            return None;
        }
        let name = string(msg_send![app, localizedName]).filter(|name| !name.is_empty())?;
        let url: *mut Object = msg_send![app, bundleURL];
        let bundle = if url.is_null() { None } else { string(msg_send![url, path]) };
        Some(Front { name, bundle })
    }
}

fn dir() -> PathBuf {
    data_dir().join("app-icons")
}

/// Where `app`'s icon is kept, whether or not it is there yet.
pub fn icon_path(app: &str) -> PathBuf {
    let file: String = app
        .chars()
        .map(|c| if c == '/' || c == ':' || c.is_control() { '_' } else { c })
        .collect();
    dir().join(format!("{file}.png"))
}

/// The kept icon, if there is one.
pub fn icon(app: &str) -> Option<PathBuf> {
    Some(icon_path(app)).filter(|path| path.is_file())
}

/// Keep `app`'s icon, from its bundle or, failing that, wherever macOS finds
/// an app by that name. Returns whether a new icon was kept.
pub fn keep_icon(app: &str, bundle: Option<&str>) -> bool {
    let path = icon_path(app);
    if path.is_file() {
        return false;
    }
    let Some(bundle) = bundle.map(str::to_string).or_else(|| find(app)) else {
        return false;
    };
    let Some(png) = objc::rc::autoreleasepool(|| unsafe { icon_png(&bundle) }) else {
        return false;
    };
    std::fs::create_dir_all(dir()).is_ok() && std::fs::write(path, png).is_ok()
}

/// An app's bundle, by its name.
fn find(app: &str) -> Option<String> {
    let found = objc::rc::autoreleasepool(|| unsafe {
        let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let path: *mut Object = msg_send![workspace, fullPathForApplication: ns_string(app)?];
        string(path)
    });
    found.or_else(|| {
        ["/Applications", "/System/Applications", "/System/Applications/Utilities"]
            .iter()
            .map(|dir| format!("{dir}/{app}.app"))
            .find(|path| std::path::Path::new(path).is_dir())
    })
}

/// The icon macOS shows for `bundle`, as a 64 px PNG.
unsafe fn icon_png(bundle: &str) -> Option<Vec<u8>> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSSize(f64, f64);
    unsafe impl objc::Encode for NSSize {
        fn encode() -> objc::Encoding {
            unsafe { objc::Encoding::from_str("{CGSize=dd}") }
        }
    }
    /// `NSBitmapImageFileTypePNG`.
    const PNG: usize = 4;

    unsafe {
        let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let image: *mut Object = msg_send![workspace, iconForFile: ns_string(bundle)?];
        if image.is_null() {
            return None;
        }
        let _: () = msg_send![image, setSize: NSSize(64., 64.)];
        let nil = std::ptr::null_mut::<Object>();
        let cg: *mut c_void =
            msg_send![image, CGImageForProposedRect: std::ptr::null_mut::<c_void>() context: nil hints: nil];
        if cg.is_null() {
            return None;
        }
        let rep: *mut Object = msg_send![class!(NSBitmapImageRep), alloc];
        let rep: *mut Object = msg_send![rep, initWithCGImage: cg];
        if rep.is_null() {
            return None;
        }
        let empty: *mut Object = msg_send![class!(NSDictionary), dictionary];
        let data: *mut Object = msg_send![rep, representationUsingType: PNG properties: empty];
        let png = if data.is_null() {
            None
        } else {
            let bytes: *const u8 = msg_send![data, bytes];
            let len: usize = msg_send![data, length];
            (!bytes.is_null() && len > 0).then(|| std::slice::from_raw_parts(bytes, len).to_vec())
        };
        let _: () = msg_send![rep, release];
        png
    }
}

unsafe fn string(ns: *mut Object) -> Option<String> {
    if ns.is_null() {
        return None;
    }
    let utf8: *const c_char = unsafe { msg_send![ns, UTF8String] };
    (!utf8.is_null()).then(|| unsafe { CStr::from_ptr(utf8) }.to_string_lossy().into_owned())
}

unsafe fn ns_string(s: &str) -> Option<*mut Object> {
    let c = CString::new(s).ok()?;
    let ns: *mut Object = unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] };
    (!ns.is_null()).then_some(ns)
}
