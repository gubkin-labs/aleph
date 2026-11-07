//! Adversarial routes: plans that pass `aleph check` at or near the route limits but do as much
//! real work per op as the language allows. The meter counts ops, not time, so these measure how
//! long and how much memory the slowest valid route actually takes.
//!
//! `cargo bench -p aleph-core --bench adversarial` prints a table (static bounds, metered usage,
//! wall time, peak heap) and then runs criterion on each case. `-- --report` prints only the
//! table.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::needless_pass_by_value,
    missing_docs
)]

use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aleph_core::manifest::Method;
use aleph_core::{Engine, FixedEnv, Limits, MemoryStore, Request, TenantId, parse_and_validate};
use criterion::Criterion;
use peak_alloc::PeakAlloc;
use serde_json::{Value as Json, json};

#[global_allocator]
static HEAP: PeakAlloc = PeakAlloc;

// Expression builders.
fn lit(v: impl Into<Json>) -> Json {
    json!({ "kind": "lit", "value": v.into() })
}
fn var(id: u32) -> Json {
    json!({ "kind": "var", "id": id })
}
fn body(name: &str) -> Json {
    json!({ "kind": "input", "source": "body", "name": name })
}
fn len(of: Json) -> Json {
    json!({ "kind": "length", "of": of })
}
fn eval(expr: Json) -> Json {
    json!({ "op": "eval", "expr": expr })
}
fn let_(id: u32, op: Json) -> Json {
    json!({ "kind": "let", "var": id, "op": op })
}
fn string(max: u32) -> Json {
    json!({ "type": { "kind": "string", "min": 0, "max": max } })
}

/// A one-route app: `POST /x` with these body inputs, statements, and output.
fn app(inputs: &Json, stmts: Vec<Json>, output: Json) -> String {
    json!({
        "formatVersion": "0.2",
        "app": { "name": "adversarial" },
        "routes": [{
            "id": "x",
            "method": "POST",
            "path": "/x",
            "auth": { "kind": "public" },
            "input": { "body": inputs },
            "plan": { "body": stmts, "output": output },
        }],
    })
    .to_string()
}

struct Case {
    name: &'static str,
    manifest: String,
    body: Json,
}

fn a(n: usize) -> String {
    "a".repeat(n)
}

/// A body string field of `chars` characters (`a`s), declared with `max` = `chars`.
fn text_input(chars: usize) -> (Json, Json) {
    (
        json!({ "s": string(u32::try_from(chars).unwrap()) }),
        json!({ "s": a(chars) }),
    )
}

/// A body array of `n` one-character strings.
fn items_input(n: usize) -> (Json, Json) {
    (
        json!({ "xs": { "type": { "kind": "array", "max": n, "items": { "kind": "string", "min": 0, "max": 1 } } } }),
        json!({ "xs": vec!["a"; n] }),
    )
}

/// `sum([e, e, ..., e])` with `n` copies: a flat way to evaluate `e` many times, since JSON
/// nesting is limited to 128 levels.
fn sum_of(e: &Json, n: usize) -> Json {
    json!({ "kind": "agg", "op": "sum", "item": 999, "list": { "kind": "list", "items": vec![e.clone(); n] } })
}

fn map(list: Json, item: u32, yield_: Json) -> Json {
    json!({ "op": "map", "list": list, "item": item, "yield": yield_ })
}

fn split(of: Json, max: u32) -> Json {
    json!({ "kind": "split", "of": of, "sep": lit(","), "max": max })
}

/// The cases. Each is an attack on one op kind; most come in two sizes: the one that was slow
/// before usage counted it (now rejected by the checker), and the largest that still passes.
#[allow(clippy::too_many_lines)]
fn cases() -> Vec<Case> {
    const MB: usize = 1 << 20;
    let mut out = Vec::new();
    let mut case =
        |name: &'static str, (inputs, body): (Json, Json), stmts: Vec<Json>, output: Json| {
            out.push(Case {
                name,
                manifest: app(&inputs, stmts, output),
                body,
            });
        };
    let none = || (json!({}), json!({}));
    let count_of = |e: &Json, n: usize| json!({ "kind": "agg", "op": "count", "item": 999, "list": { "kind": "list", "items": vec![e.clone(); n] } });

    case("baseline/return literal", none(), vec![], lit(1));

    // Reading a value copies it: `input`, `var`, and `lit` each cost 1 op whatever their size.
    case(
        "clone/1 MB string x660",
        text_input(MB),
        vec![],
        sum_of(&len(body("s")), 660),
    );
    case(
        "clone/12 KB string x660",
        text_input(12 * 1024),
        vec![],
        sum_of(&len(body("s")), 660),
    );
    case(
        "clone/200k-item list x660",
        items_input(200_000),
        vec![],
        sum_of(&len(body("xs")), 660),
    );
    case(
        "clone/100-item list x660",
        items_input(100),
        vec![],
        sum_of(&len(body("xs")), 660),
    );
    case(
        "clone/let 200k-item list, var x660",
        items_input(200_000),
        vec![let_(0, eval(body("xs")))],
        sum_of(&len(var(0)), 660),
    );
    case(
        "clone/let 100-item list, var x660",
        items_input(100),
        vec![let_(0, eval(body("xs")))],
        sum_of(&len(var(0)), 660),
    );

    // Building: `concat` and `listConcat` results.
    case(
        "concat/64 KB string x1990",
        text_input(64 * 1024),
        vec![],
        len(json!({ "kind": "concat", "parts": vec![body("s"); 1990] })),
    );
    case(
        "concat/512 chars x995",
        text_input(512),
        vec![],
        len(json!({ "kind": "concat", "parts": vec![body("s"); 995] })),
    );
    case(
        "listConcat/10k-item list x200",
        items_input(10_000),
        vec![],
        len(json!({ "kind": "listConcat", "parts": vec![body("xs"); 200] })),
    );
    case(
        "listConcat/60-item list x990",
        items_input(60),
        vec![],
        len(json!({ "kind": "listConcat", "parts": vec![body("xs"); 990] })),
    );
    // `replace` used to build the whole result before checking the 1 MB cap.
    case(
        "replace/1k chars, with 64 KB (5 ops)",
        text_input(1000),
        vec![],
        len(
            json!({ "kind": "replace", "of": body("s"), "find": lit("a"), "with": lit(a(64 * 1024)) }),
        ),
    );
    case(
        "replace/1 MB chars, with 1 MB (5 ops)",
        text_input(MB),
        vec![],
        len(json!({ "kind": "replace", "of": body("s"), "find": lit("a"), "with": lit(a(MB)) })),
    );
    case(
        "replace/1k chars, with 1k (result 1 MB)",
        text_input(1000),
        vec![],
        len(json!({ "kind": "replace", "of": body("s"), "find": lit("a"), "with": lit(a(1000)) })),
    );
    case(
        "str/upper 200 KB x2",
        text_input(200 * 1024),
        vec![],
        sum_of(
            &len(json!({ "kind": "str", "op": "upper", "of": body("s") })),
            2,
        ),
    );
    case(
        "slice/300 KB string x2",
        text_input(300 * 1024),
        vec![],
        sum_of(
            &len(json!({ "kind": "slice", "of": body("s"), "start": lit(1) })),
            2,
        ),
    );

    // Regular expressions: matching is linear in the text, times the pattern's states.
    let class = format!("(?:a|b|c|d|e|f|g|h|i|j)*{}z", "[a-z]".repeat(40));
    let matches = |p: &str| json!({ "kind": "matches", "of": body("s"), "pattern": p });
    case(
        "matches/1 MB, a{0,250}b x1",
        text_input(MB),
        vec![],
        matches("a{0,250}b"),
    );
    case(
        "matches/64 KB, 256-char pattern x660",
        text_input(64 * 1024),
        vec![],
        count_of(&matches(&class), 660),
    );
    case(
        "matches/64 KB, 256-char pattern x1",
        text_input(64 * 1024),
        vec![],
        matches(&class),
    );
    case(
        "matches/3 KB, 256-char pattern x1",
        text_input(3000),
        vec![],
        matches(&class),
    );
    case(
        "matches/240 chars, a{0,500}b (1003 states)",
        text_input(240),
        vec![],
        matches("a{0,500}b"),
    );
    case(
        "matches/900 chars, a x256",
        text_input(900),
        vec![],
        matches(&a(256)),
    );
    case(
        "matches/20 chars, 256-char pattern x90",
        text_input(20),
        vec![],
        count_of(&matches(&class), 90),
    );
    let email = r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$";
    case(
        "matches/254-char email x1",
        text_input(254),
        vec![],
        matches(email),
    );

    // Sorting and de-duplicating strings with a long shared prefix.
    let long = |n: usize| {
        (
            json!({ "xs": { "type": { "kind": "array", "max": n, "items": { "kind": "string", "min": 0, "max": 1000 } } } }),
            json!({ "xs": (0..n).map(|i| format!("{}{i:04}", a(990))).collect::<Vec<_>>() }),
        )
    };
    case(
        "sort/150 x 1 KB strings",
        long(150),
        vec![],
        len(json!({ "kind": "sort", "list": body("xs"), "item": 1, "by": var(1), "desc": true })),
    );
    case(
        "distinct/600 x 1 KB strings",
        long(600),
        vec![],
        len(json!({ "kind": "distinct", "of": body("xs") })),
    );
    case(
        "split/100 chars into 100 x40",
        (json!({ "s": string(100) }), json!({ "s": ",".repeat(99) })),
        vec![],
        sum_of(&len(split(body("s"), 100)), 40),
    );

    // Map: each item's body reads a big value.
    case(
        "map/600 items reading 1 KB string",
        {
            let (mut i, mut b) = text_input(1000);
            let (xi, xb) = items_input(600);
            i["xs"] = xi["xs"].clone();
            b["xs"] = xb["xs"].clone();
            (i, b)
        },
        vec![let_(0, map(body("xs"), 1, len(body("s"))))],
        lit(1),
    );

    // Decimal and time builtins in bulk.
    let dec = json!({ "kind": "binary", "op": "mul", "lhs": { "kind": "decimal", "value": "12345678.123456789" }, "rhs": { "kind": "decimal", "value": "0.000000001" } });
    case(
        "decimal/mul x660",
        none(),
        vec![],
        len(json!({ "kind": "list", "items": vec![dec; 660] })),
    );
    let time = json!({ "kind": "formatTime", "of": { "kind": "now" }, "pattern": "%A %d %B %Y %H:%M:%S %Z", "tz": lit("America/New_York") });
    case(
        "time/formatTime tz x660",
        none(),
        vec![],
        len(json!({ "kind": "list", "items": vec![time; 660] })),
    );

    // `in` against a big literal list.
    let consts = |n: usize| -> Vec<Json> { (0..n).map(|i| json!(format!("value-{i}"))).collect() };
    let inn = |n: usize| json!({ "kind": "in", "value": lit("missing"), "list": consts(n) });
    case(
        "in/100k literals x990",
        none(),
        vec![],
        count_of(&inn(100_000), 990),
    );
    case(
        "in/64 literals x990",
        none(),
        vec![],
        count_of(&inn(64), 990),
    );
    case("in/100k literals x1", none(), vec![], inn(100_000));

    // A loop with big state.
    case(
        "loop/600 passes, 1.7 KB state",
        none(),
        vec![let_(
            0,
            json!({ "op": "loop", "max": 600, "var": 1,
                "state": { "t": lit(a(1700)) },
                "next": { "t": field(var(1), "t") } }),
        )],
        lit(1),
    );
    case(
        "loop/300 passes, 2 KB state, concat",
        none(),
        vec![let_(
            0,
            json!({ "op": "loop", "max": 300, "var": 1,
                "state": { "t": lit(a(2000)) },
                "next": { "t": json!({ "kind": "truncate", "max": 2000, "of": { "kind": "concat", "parts": [field(var(1), "t"), lit("b")] } }) } }),
        )],
        lit(1),
    );

    out
}

fn field(of: Json, name: &str) -> Json {
    json!({ "kind": "field", "of": of, "name": name })
}

fn engine(manifest: &str) -> Engine {
    let valid = match parse_and_validate(manifest, &Limits::ROUTE) {
        Ok(v) => v,
        Err(diags) => panic!("doesn't validate: {diags:#?}"),
    };
    Engine::new(
        valid,
        Arc::new(MemoryStore::new()),
        Arc::new(FixedEnv::new(1_700_000_000_000, 1)),
    )
}

fn request(body: &Json) -> Request {
    Request {
        method: Method::Post,
        path: "/x".to_owned(),
        query: BTreeMap::new(),
        body: Some(body.clone()),
        caller: None,
        ip: None,
        idempotency_key: None,
        headers: BTreeMap::new(),
        raw_body: None,
    }
}

fn report(cases: &[Case]) {
    println!(
        "{:<44} {:>6} {:>10} {:>6} {:>10} {:>5} {:>10} {:>10} {:>10}",
        "case", "ops≤", "memory≤", "ops", "memory", "st", "check", "time", "peak heap"
    );
    for case in cases {
        let started = Instant::now();
        let valid = parse_and_validate(&case.manifest, &Limits::ROUTE);
        let check = started.elapsed();
        let bounds = match &valid {
            Ok(v) => v.report().routes["x"].bounds,
            Err(d) => {
                println!("{:<44} INVALID: {}", case.name, d[0].message);
                continue;
            }
        };
        let engine = engine(&case.manifest);
        let tenant = TenantId::new("bench").unwrap();
        let req = request(&case.body);
        let before = HEAP.current_usage();
        HEAP.reset_peak_usage();
        let started = Instant::now();
        let r = engine.handle(&tenant, &req);
        let time = started.elapsed();
        let peak = HEAP.peak_usage().saturating_sub(before);
        println!(
            "{:<44} {:>6} {:>10} {:>6} {:>10} {:>5} {:>10} {:>10} {:>10}",
            case.name,
            bounds.ops,
            bounds.memory,
            r.usage.ops,
            r.usage.memory,
            r.status,
            fmt(check),
            fmt(time),
            bytes(peak),
        );
        if r.status >= 400 {
            println!("    {}", r.body);
        }
    }
}

fn fmt(d: Duration) -> String {
    if d.as_millis() >= 10 {
        format!("{} ms", d.as_millis())
    } else {
        format!("{} µs", d.as_micros())
    }
}

fn bytes(n: usize) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / f64::from(1 << 20))
    } else {
        format!("{:.1} KB", n as f64 / 1024.0)
    }
}

fn main() {
    let cases = cases();
    report(&cases);
    if std::env::args().any(|a| a == "--report") {
        return;
    }
    let mut c = Criterion::default().configure_from_args();
    let tenant = TenantId::new("bench").unwrap();
    for case in &cases {
        if parse_and_validate(&case.manifest, &Limits::ROUTE).is_err() {
            continue;
        }
        let engine = engine(&case.manifest);
        let req = request(&case.body);
        c.bench_function(&format!("run/{}", case.name), |b| {
            b.iter(|| black_box(engine.handle(&tenant, &req).status));
        });
        c.bench_function(&format!("check/{}", case.name), |b| {
            b.iter(|| black_box(parse_and_validate(&case.manifest, &Limits::ROUTE).is_ok()));
        });
    }
    c.final_summary();
}
