mod probe;
mod render;
mod replay;
mod sanitize;

// gpui-component streaming-markdown spike. Modes:
//   (default)     Task 1 static render (feasibility smoke)
//   --stream      Task 5 streaming render + probe over the GFM stress fixture
// See NOTES.md for the discovered API and findings.

use gpui::{App, Application, Context, Window, WindowOptions, div, prelude::*};
use gpui_component::{Root, text::TextView};

use render::{Source, StreamView};

const SAMPLE: &str = "# Hello\n\nSome **bold** and a list:\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n[a link](https://example.com)\n";

struct MdView;

impl Render for MdView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_4()
            .child(TextView::markdown("md", SAMPLE, window, cx))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let source = if args.iter().any(|a| a == "--adversarial") {
        Some(Source::Adversarial)
    } else if args.iter().any(|a| a == "--big") {
        Some(Source::Big)
    } else if args.iter().any(|a| a == "--stream") {
        Some(Source::Stress)
    } else {
        None
    };
    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        cx.open_window(WindowOptions::default(), move |window, cx| {
            let any: gpui::AnyView = match source {
                Some(src) => cx.new(move |cx| StreamView::new(src, cx)).into(),
                None => cx.new(|_| MdView).into(),
            };
            cx.new(|cx| Root::new(any, window, cx))
        })
        .unwrap();
        cx.activate(true);
    });
}
