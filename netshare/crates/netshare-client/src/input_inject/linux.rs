//! Linux uinput injection — stub for later.
use netshare_core::input::{KeyEvent, MouseClick, MouseMove, MouseScroll};

pub fn inject_mouse_move(_: MouseMove)  { todo!("uinput inject — Linux") }
pub fn inject_mouse_click(_: MouseClick) { todo!("uinput inject — Linux") }
pub fn inject_scroll(_: MouseScroll)    { todo!("uinput inject — Linux") }
pub fn inject_key(_: KeyEvent)          { todo!("uinput inject — Linux") }
