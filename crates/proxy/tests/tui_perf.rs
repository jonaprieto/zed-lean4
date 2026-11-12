//! Render and input cost of the terminal infoview on large goals and messages.
//! Run with `cargo test --release -p lean4-ide-proxy --test tui_perf -- --ignored --nocapture`.
use lean4_ide_proxy::tui::app::App;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const U: &str = "file:///p/Big.lean";

/// A term of `n` tagged tokens, broken by the pretty printer every 12 tokens.
fn term(n: usize, seed: usize) -> Value {
    let parts: Vec<Value> = (0..n)
        .map(|i| {
            let sep = if i % 12 == 11 { "\n    " } else { " " };
            json!({"append":[
                {"tag":[{"info":{"__rpcref":format!("{seed}-{i}")}},{"text":format!("Foo.bar_{i} x{i} (∀ y, y + {i} = y)")}]},
                {"text":sep},
            ]})
        })
        .collect();
    json!({"append": parts})
}

fn goals(hyps: usize, tokens: usize) -> Value {
    let goal = |g: usize| {
        let hyps: Vec<Value> = (0..hyps)
            .map(|h| json!({"names":[format!("h{h}")],"type":term(tokens, g * 1000 + h)}))
            .collect();
        json!({"userName":format!("case{g}"),"hyps":hyps,"type":term(tokens, g)})
    };
    json!({"goals":[goal(0), goal(1), goal(2)]})
}

/// One message holding a trace tree `depth` deep with `width` children per node.
fn trace(depth: usize, width: usize, path: &str) -> Value {
    let children: Vec<Value> = if depth == 0 {
        vec![]
    } else {
        (0..width)
            .map(|i| trace(depth - 1, width, &format!("{path}.{i}")))
            .collect()
    };
    json!({"tag":[{"trace":{"indent":depth,"cls":"Meta.isDefEq","msg":{"append":[
        {"text":format!("✅ {path} ")},
        {"tag":[{"expr":term(4, depth)},{"text":""}]},
    ]},"collapsed":false,"children":{"strict":children}}},{"text":""}]})
}

fn time<T>(runs: u32, mut f: impl FnMut() -> T) -> Duration {
    let start = Instant::now();
    for _ in 0..runs {
        std::hint::black_box(f());
    }
    start.elapsed() / runs
}

#[test]
#[ignore = "performance measurement; run in release with --nocapture"]
fn large_goals_and_messages_render_within_a_frame() {
    let mut app = App::new("tui-1".into());
    app.connected("http://127.0.0.1:1/?token=t".into());
    let loc =
        json!({"uri":U,"range":{"start":{"line":3,"character":2},"end":{"line":3,"character":2}}});
    app.on_message(&json!({"id":"tui-1:1","result":{"location":loc,"initializeResult":{}}}));
    app.on_message(&json!({"id":"tui-1:3","result":"s1"}));

    // 3 goals x 40 hypotheses x 60 tagged tokens: about 7,200 subterms.
    let big = json!({"id":"tui-1:4","result":goals(40, 60)});
    let start = Instant::now();
    app.on_message(&big);
    let goals_reply = start.elapsed();
    // A trace tree of 4^5 = 1,365 nodes plus 30 one-line messages.
    let mut diags: Vec<Value> = (0..30)
        .map(|i| {
            json!({"range":{"start":{"line":3,"character":i},"end":{"line":3,"character":i}},
            "severity":2,"message":{"text":format!("message {i}")}})
        })
        .collect();
    diags.push(
        json!({"range":{"start":{"line":3,"character":0},"end":{"line":3,"character":0}},
        "severity":3,"message":trace(5, 4, "t")}),
    );
    let start = Instant::now();
    app.on_message(&json!({"id":"tui-1:6","result":diags}));
    let diags_reply = start.elapsed();

    let mut terminal = Terminal::new(TestBackend::new(120, 50)).unwrap();
    let render = time(20, || terminal.draw(|f| app.render(f)).unwrap().area);
    let key = time(200, || {
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
    });
    let fold = time(20, || {
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    });
    terminal.draw(|f| app.render(f)).unwrap();
    let click = time(50, || {
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 30,
            row: 10,
            modifiers: KeyModifiers::NONE,
        })
    });
    let wheel = time(50, || {
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })
    });
    println!(
        "goals reply {goals_reply:?}, diagnostics reply {diags_reply:?}, render {render:?}, \
         key {key:?}, fold {fold:?}, click {click:?}, wheel {wheel:?}"
    );
    // One 60 Hz frame for anything that blocks a keystroke or a redraw.
    let frame = Duration::from_millis(16);
    if !cfg!(debug_assertions) {
        for (what, cost) in [
            ("render", render),
            ("key", key),
            ("fold", fold),
            ("click", click),
        ] {
            assert!(cost < frame, "{what} takes {cost:?}");
        }
    }
}
