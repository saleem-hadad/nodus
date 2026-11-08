//! The proof of our GPUI patch set (third_party/gpui/PATCHES.md).

use gpui_kit::*;

struct Effects;

impl Render for Effects {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div().size_full().child("effects")
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Effects))
            .expect("the window opens");
    });
}
