use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, sel};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

pub(crate) fn visual_top_left(contents_are_flipped: bool) -> &'static std::ffi::CStr {
    if contents_are_flipped {
        c"bottomLeft"
    } else {
        c"topLeft"
    }
}

pub(crate) fn anchor_top_left(window: &Window) -> usize {
    let Ok(handle) = window.window_handle() else {
        return 0;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return 0;
    };
    let (Some(metal), Some(string)) = (AnyClass::get("CAMetalLayer"), AnyClass::get("NSString"))
    else {
        return 0;
    };
    let view = appkit.ns_view.as_ptr().cast::<AnyObject>();
    #[allow(unsafe_code)]
    unsafe {
        let root: *mut AnyObject = msg_send![view, layer];
        if root.is_null() {
            return 0;
        }
        let mut layers = vec![root];
        let sublayers: *mut AnyObject = msg_send![root, sublayers];
        if !sublayers.is_null() {
            let count: usize = msg_send![sublayers, count];
            for i in 0..count {
                let layer: *mut AnyObject = msg_send![sublayers, objectAtIndex: i];
                layers.push(layer);
            }
        }
        let mut anchored = 0;
        for layer in layers {
            let is_metal: bool = msg_send![layer, isKindOfClass: metal];
            let settable: bool = msg_send![layer, respondsToSelector: sel!(setContentsGravity:)];
            if !(is_metal && settable) {
                continue;
            }
            let flipped: bool = msg_send![layer, contentsAreFlipped];
            let name = visual_top_left(flipped);
            let gravity: *mut AnyObject = msg_send![string, stringWithUTF8String: name.as_ptr()];
            if gravity.is_null() {
                continue;
            }
            let () = msg_send![layer, setContentsGravity: gravity];
            anchored += 1;
        }
        anchored
    }
}

#[cfg(test)]
mod tests {
    use super::visual_top_left;

    #[test]
    fn a_flipped_layer_takes_bottom_left_to_anchor_at_the_visual_top_left() {
        assert_eq!(visual_top_left(true), c"bottomLeft");
        assert_eq!(visual_top_left(false), c"topLeft");
    }
}
