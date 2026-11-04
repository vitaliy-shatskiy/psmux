// A frame carries the title the pane program set last.
//
// `set-titles-string "#{pane_title}"` expands `pane.title`, which follows the
// parser's OSC 0/2 title only through `propagate_osc_titles`. The pushed frame
// did not call it, and an attached client that keeps receiving pushes (a pane
// printing all the time) never sends the dump-state that does, so the host
// terminal kept the old title for as long as the pane kept printing.

use super::{build_frame, FrameCache};

fn set_osc_title(app: &mut crate::types::AppState, title: &str) {
    let win = &mut app.windows[0];
    let pane = crate::tree::active_pane_mut(&mut win.root, &win.active_path).expect("the pane");
    pane.term.lock().expect("the parser").process(format!("\x1b]2;{title}\x07").as_bytes());
}

#[test]
fn a_frame_carries_the_title_the_pane_set_since_the_last_frame() {
    let mut app = crate::server::test_issue431_m4_blobs::app_with_sixel_pane();
    app.set_titles = true;
    app.allow_set_title = true;
    app.set_titles_string = "#{pane_title}".to_string();
    let mut cache = FrameCache::new();
    let mut meta_dirty = true;

    for title in ["first title", "second title"] {
        set_osc_title(&mut app, title);
        build_frame(&mut app, &mut cache, &mut meta_dirty).expect("build_frame");
        let frame: serde_json::Value =
            serde_json::from_str(&cache.combined_buf).expect("the frame is JSON");
        assert_eq!(frame["host_title"], title, "the frame's host title");
        // The pane border draws the layout's title, a cached frame is all an
        // idle client sees: both titles must come from the same parse.
        let mut titles = Vec::new();
        collect_pane_titles(&frame["layout"], &mut titles);
        assert_eq!(titles, [title], "the pane title inside the frame's layout");
    }
}

fn collect_pane_titles<'a>(node: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    if let Some(title) = node["title"].as_str() {
        out.push(title);
    }
    if let Some(children) = node["children"].as_array() {
        for child in children {
            collect_pane_titles(child, out);
        }
    }
}
