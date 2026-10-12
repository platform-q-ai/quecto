//! The spawn check's corpus: every form the reader is held to, with the
//! sites it finds in each (`every_spawn_form_is_seen`; none: no spawn),
//! and the proof that it catches every form the check as merged (#2513)
//! caught (`the_reader_catches_all_the_merged_check_caught`), but the
//! false positives it was written to leave ([`MERGED_FALSE_POSITIVES`]).

use super::master::master_sites;
use super::spawn_sites;

/// Each form, and what the reader finds in it, site by site.
const CASES: &[(&str, &[&str])] = &[
    (
        r#"fn f() { tokio::task::spawn_blocking(|| ()); }"#,
        &[r#"`tokio::task::spawn_blocking`"#],
    ),
    (
        r#"use tokio::task::spawn_blocking; fn f() { spawn_blocking(|| ()); }"#,
        &[r#"`tokio::task::spawn_blocking`"#, r#"`spawn_blocking`"#],
    ),
    (
        r#"use tokio::task::{self as t}; fn f() { t::spawn_blocking(|| ()); }"#,
        &[
            r#"a `use` renaming or globbing `tokio::task`"#,
            r#"`t::spawn_blocking`"#,
        ],
    ),
    (
        r#"fn f() { std::thread::spawn(|| ()); }"#,
        &[r#"`std::thread::spawn`"#],
    ),
    (
        r#"use std::thread; fn f() { thread::spawn(|| ()); }"#,
        &[r#"`thread::spawn`"#],
    ),
    (
        r#"fn f() { tokio::runtime::Handle::current().spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `tokio :: runtime :: Handle :: current ()`"#],
    ),
    (
        r#"fn f(h: tokio::runtime::Handle) { h.block_on(async {}); }"#,
        &[r#"`.block_on(..)` on `h`"#],
    ),
    (
        r#"fn f() { tokio::task::spawn_local(async {}); }"#,
        &[r#"`tokio::task::spawn_local`"#],
    ),
    (
        r#"fn f() { std::thread::Builder::new().spawn(|| ()).unwrap(); }"#,
        &[r#"`.spawn(..)` on `std :: thread :: Builder :: new ()`"#],
    ),
    (
        r#"fn f() { tokio::spawn(async {}); }"#,
        &[r#"`tokio::spawn`"#],
    ),
    (
        r#"fn f() { let _ = format!("{:?}", tokio::spawn(async {})); }"#,
        &[r#"`spawn` in a macro"#],
    ),
    (
        r#"impl X { fn f(&self) { self.handle.spawn(async {}); } }"#,
        &[r#"`.spawn(..)` on `self . handle`"#],
    ),
    (
        r#"fn f() { tokio::task::JoinSet::<()>::spawn; }"#,
        &[r#"`tokio::task::JoinSet::spawn`"#],
    ),
    (
        r#"fn f() { let mut set = tokio::task::JoinSet::new(); set.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `set`"#],
    ),
    (
        r#"use tokio::task::JoinSet; fn f() { let mut s = JoinSet::new(); s.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `s`"#],
    ),
    (
        r#"fn f() { std::thread::scope(|s| { s.spawn(|| ()); }); }"#,
        &[r#"`std::thread::scope`"#],
    ),
    (
        r#"use std::thread; fn f() { thread::scope(|s| ()); }"#,
        &[r#"`thread::scope`"#],
    ),
    (
        r#"fn f(pool: &ThreadPool) { pool.spawn_ok(async {}); }"#,
        &[r#"`.spawn_ok(..)` on `pool`"#],
    ),
    (r#"fn f() { rayon::spawn(|| ()); }"#, &[r#"`rayon::spawn`"#]),
    (
        r#"fn f(builder: std::thread::Builder) { builder.spawn(|| ()).unwrap(); }"#,
        &[r#"`.spawn(..)` on `builder`"#],
    ),
    (
        r#"fn f() { let h = tokio::runtime::Handle::current(); h.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { let rt = tokio::runtime::Runtime::new().unwrap(); rt.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `rt`"#],
    ),
    (
        r#"use tokio as tk; fn f() { tk::spawn(async {}); }"#,
        &[r#"a `use` renaming or globbing `tokio`"#],
    ),
    (
        r#"use tokio::task as t; fn f() { t::spawn(async {}); }"#,
        &[r#"a `use` renaming or globbing `tokio::task`"#],
    ),
    (
        r#"use tokio::runtime as r; fn f() {}"#,
        &[r#"a `use` renaming or globbing `tokio::runtime`"#],
    ),
    (
        r#"use std::thread as th; fn f() { th::spawn(|| ()); }"#,
        &[r#"a `use` renaming or globbing `std::thread`"#],
    ),
    (
        r#"use tokio::*; fn f() { spawn(async {}); }"#,
        &[r#"a `use` renaming or globbing `tokio`"#],
    ),
    (
        r#"use tokio::task::*; fn f() { spawn(async {}); }"#,
        &[r#"a `use` renaming or globbing `tokio::task`"#],
    ),
    (
        r#"use std::thread::*; fn f() { spawn(|| ()); }"#,
        &[r#"a `use` renaming or globbing `std::thread`"#],
    ),
    (
        r#"use tokio::runtime::Handle as H; fn f() { H::current().spawn(async {}); }"#,
        &[r#"a `use` renaming or globbing `tokio::runtime::Handle`"#],
    ),
    (
        r#"fn f() { drop(format!("{:?}", tokio::runtime::Handle::current().spawn(async {}))); }"#,
        &[r#"`spawn` in a macro"#],
    ),
    (
        r#"fn f() { let rt = Runtime::new().unwrap(); drop(format!("{:?}", rt.spawn(async {}))); }"#,
        &[r#"`spawn` in a macro"#],
    ),
    (
        r#"fn f() { if let Ok(h) = tokio::runtime::Handle::try_current() { h.spawn(async {}); } }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { while let Some(h) = Handle::try_current().ok() { h.spawn(async {}); } }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { match Handle::try_current() { Ok(h) => { h.spawn(async {}); } Err(_) => {} } }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { Handle::try_current().map(|h| h.spawn(async {})); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { let g = |h: Handle| h.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"use tokio::runtime::Runtime as R; fn f() {}"#,
        &[r#"a `use` renaming or globbing `tokio::runtime::Runtime`"#],
    ),
    (
        r#"use tokio::task::JoinSet as Set; fn f() {}"#,
        &[r#"a `use` renaming or globbing `tokio::task::JoinSet`"#],
    ),
    (
        r#"use std::thread::Builder as B; fn f() {}"#,
        &[r#"a `use` renaming or globbing `std::thread::Builder`"#],
    ),
    (
        r#"use tokio::runtime::{Builder as B, Handle}; fn f() {}"#,
        &[r#"a `use` renaming or globbing `tokio::runtime::Builder`"#],
    ),
    (
        r#"type H = tokio::runtime::Handle; fn f() {}"#,
        &[r#"`type H`, an alias of a spawner"#],
    ),
    (
        r#"fn f() { type S = JoinSet<()>; }"#,
        &[r#"`type S`, an alias of a spawner"#],
    ),
    (
        r#"extern crate tokio as tk; fn f() {}"#,
        &[r#"`extern crate tokio` renamed"#],
    ),
    (
        r#"extern crate rayon as r; fn f() {}"#,
        &[r#"`extern crate rayon` renamed"#],
    ),
    (
        r#"fn f(h: Handle) { let h2 = h.clone(); h2.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h2`"#],
    ),
    (
        r#"impl X { fn f(&self) { let h = self.handle.clone(); h.spawn(async {}); } }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"impl X { fn f(&self) { let r = self.rt.clone(); let s = r; s.spawn(async {}); } fn g(rt: Runtime) { let a = rt; a.spawn(async {}); } }"#,
        &[r#"`.spawn(..)` on `a`"#],
    ),
    (
        r#"fn f(set: &mut JoinSet<()>) { set.spawn_on(async {}, &h); }"#,
        &[r#"`.spawn_on(..)` on `set`"#],
    ),
    (
        r#"fn f() { tokio::task::spawn_blocking_on(|| (), &h); }"#,
        &[r#"`tokio::task::spawn_blocking_on`"#],
    ),
    (
        r#"fn f() { tokio::task::spawn_local_on(async {}, &h); }"#,
        &[r#"`tokio::task::spawn_local_on`"#],
    ),
    (
        r#"fn f() { <Handle>::spawn(&h, async {}); }"#,
        &[r#"`Handle::spawn`"#],
    ),
    (
        r#"fn f() { <Handle as Clone>::clone(&h).spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `< Handle as Clone > :: clone (& h)`"#],
    ),
    (
        r#"fn f() { let mut s: JoinSet<()> = Default::default(); s.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `s`"#],
    ),
    (
        r#"fn f() { drop(format!("{:?}", tokio::task::JoinSet::<()>::spawn)); }"#,
        &[r#"`spawn` in a macro"#],
    ),
    (
        r#"fn f() { drop(format!("{:?}", JoinSet::<Vec<()>>::new().spawn(async {}))); }"#,
        &[r#"`spawn` in a macro"#],
    ),
    (
        r#"fn f(h: &Handle) { let _g = h.enter(); }"#,
        &[r#"`.enter(..)` on `h`"#],
    ),
    (
        r#"fn f() { let _g = tokio::runtime::Handle::current().enter(); }"#,
        &[r#"`.enter(..)` on `tokio :: runtime :: Handle :: current ()`"#],
    ),
    (
        r#"fn f() { let h = std::sync::Arc::new(tokio::runtime::Handle::current()); h.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { let h = Box::new(Handle::current()); h.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { let h = Some(Handle::current()); h.unwrap().spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h . unwrap ()`"#],
    ),
    (
        r#"fn f() -> std::io::Result<()> { let rt = std::sync::Arc::new(tokio::runtime::Runtime::new()?); rt.spawn(async {}); Ok(()) }"#,
        &[r#"`.spawn(..)` on `rt`"#],
    ),
    (
        r#"fn f(h: Handle) { let g = identity(h); g.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `g`"#],
    ),
    (
        r#"fn f() { std::sync::Arc::new(Handle::current()).spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `std :: sync :: Arc :: new (Handle :: current ())`"#],
    ),
    (
        r#"impl X { fn f(&self, handle: &str) { self.handle.spawn(async {}); } }"#,
        &[r#"`.spawn(..)` on `self . handle`"#],
    ),
    (
        r#"fn f() { let h = Handle::current(); { let h = 1; } h.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { let h = Handle::current(); let g = |h: u8| h; h.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f(x: Option<u8>) { let h = Handle::current(); match x { Some(h) => {} None => {} } h.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f(x: Option<u8>) { let h = Handle::current(); if let Some(h) = x {} h.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f(x: Option<u8>) { let h = Handle::current(); while let Some(h) = x {} h.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { tokio::r#spawn(async {}); }"#,
        &[r#"`tokio::spawn`"#],
    ),
    (
        r#"fn f() { tokio::task::r#spawn_blocking(|| ()); }"#,
        &[r#"`tokio::task::spawn_blocking`"#],
    ),
    (
        r#"fn f(h: Handle) { h.r#spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { drop(format!("{:?}", tokio::r#spawn(async {}))); }"#,
        &[r#"`spawn` in a macro"#],
    ),
    (
        r#"fn f() { let h; h = Handle::current(); h.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f(handles: Vec<Handle>) { for h in &handles { h.spawn(async {}); } }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f(s: S) { let S { handle, .. } = s; handle.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f(s: S) { let S { runtime: r, .. } = s; r.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `r`"#],
    ),
    (
        r#"static RT: std::sync::LazyLock<tokio::runtime::Runtime> = std::sync::LazyLock::new(make); fn f() {}"#,
        &[r#"`static RT` of a spawner type"#],
    ),
    (
        r#"const H: Option<Handle> = None; fn f() {}"#,
        &[r#"`const H` of a spawner type"#],
    ),
    (
        r#"static P: OnceLock<ThreadPool> = OnceLock::new(); fn f() {}"#,
        &[r#"`static P` of a spawner type"#],
    ),
    (
        r#"fn f(b: std::thread::Builder) { b.spawn_scoped(s, || ()).unwrap(); }"#,
        &[r#"`.spawn_scoped(..)` on `b`"#],
    ),
    (
        r#"fn f(b: std::thread::Builder) { drop(b.spawn_unchecked(|| ())); }"#,
        &[r#"`.spawn_unchecked(..)` on `b`"#],
    ),
    (
        r#"fn f() { let client = reqwest::Client::builder().build().unwrap(); client.spawn(1); }"#,
        &[],
    ),
    (r#"fn f() { Client::builder().spawn(); }"#, &[]),
    (
        r#"fn f() { let h = Handle::current(); drop(h); let h = 1; h.spawn(); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f(runtime: &str) { std::process::Command::new(runtime).spawn().unwrap(); }"#,
        &[],
    ),
    (
        r#"fn f() { let mut builder = std::process::Command::new("x"); builder.spawn().unwrap(); }"#,
        &[],
    ),
    (
        r#"fn f(builder: &mut std::process::Command) { builder.spawn().unwrap(); }"#,
        &[],
    ),
    (
        r#"fn f() { let handle = std::process::Command::new("x").spawn().unwrap(); handle.wait(); }"#,
        &[],
    ),
    (r#"use std::process::Command as C; fn f() {}"#, &[]),
    (
        r#"fn f(c: &mut std::process::Command) { c.spawn().unwrap(); }"#,
        &[],
    ),
    (
        r#"fn f() { let mut c = std::process::Command::new("x"); c.spawn().unwrap(); }"#,
        &[],
    ),
    (r#"use std::thread::sleep; fn f() {}"#, &[]),
    (
        r#"fn f() { call_work::spawn_blocking_in_call(|| ()); }"#,
        &[],
    ),
    (
        r#"#[cfg(test)] mod tests { fn f() { tokio::spawn(async {}); } }"#,
        &[],
    ),
    (
        r#"fn f(rt: &R) { rt.runtime().spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `rt . runtime ()`"#],
    ),
    (
        r#"fn f() { runtime::handle().spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `runtime :: handle ()`"#],
    ),
    (
        r#"fn f() { make_runtime().block_on(async {}); }"#,
        &[r#"`.block_on(..)` on `make_runtime ()`"#],
    ),
    (
        r#"fn f(c: &Ctx) { c.handle().spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `c . handle ()`"#],
    ),
    (
        r#"fn f() { handle().spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `handle ()`"#],
    ),
    (
        r#"fn f(c: &Ctx) { c.runtime().block_on(async {}); }"#,
        &[r#"`.block_on(..)` on `c . runtime ()`"#],
    ),
    (
        r#"fn f<H: Spawn>(handle: H) { handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f(handle: impl Spawn) { handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f(runtime: &dyn Spawn) { runtime.spawn(1); }"#,
        &[r#"`.spawn(..)` on `runtime`"#],
    ),
    (
        r#"fn f(handle: Box<Self>) { handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f() { let g = |handle| handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f() { let handle = make(); handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f(x: X) { let thread = x.worker(); thread.spawn(1); }"#,
        &[r#"`.spawn(..)` on `thread`"#],
    ),
    (
        r#"fn f() { let builder = std::thread::Builder::new(); builder.spawn(|| ()); }"#,
        &[r#"`.spawn(..)` on `builder`"#],
    ),
    (
        r#"fn f() { let handle = Default::default(); handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f() { let handle = Default::default(); handle.enter(); }"#,
        &[r#"`.enter(..)` on `handle`"#],
    ),
    (
        r#"fn f<H>(handle: Option<H>) { handle.unwrap().spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle . unwrap ()`"#],
    ),
    (
        r#"fn f(Ctx { handle, .. }: Ctx) { handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f() { let (handle, _) = pair(); handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f() { for handle in handles() { handle.spawn(1); } }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f() { if let Some(handle) = maybe() { handle.spawn(1); } }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f() { match maybe() { Some(runtime) => runtime.block_on(x), None => {} } }"#,
        &[r#"`.block_on(..)` on `runtime`"#],
    ),
    (
        r#"fn f() { while let Some(handle) = next() { handle.spawn(1); } }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f() { let handle; handle = make(); handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f(v: Vec<u8>) { v.into_iter().collect::<JoinSet<_>>().spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `v . into_iter () . collect :: < JoinSet < _ > > ()`"#],
    ),
    (
        r#"fn f(x: X) { (x as Handle).spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `(x as Handle)`"#],
    ),
    (
        r#"fn f(h: Handle) { { h }.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `{ h }`"#],
    ),
    (
        r#"fn f(c: bool, h: Handle, g: Handle) { if c { h } else { g }.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `if c { h } else { g }`"#],
    ),
    (
        r#"fn f(x: Option<Handle>, g: Handle) { match x { Some(h) => h, None => g }.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `match x { Some (h) => h , None => g }`"#],
    ),
    (
        r#"fn f(s: &S) { foo(s.handle).spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `foo (s . handle)`"#],
    ),
    (
        r#"fn f() { wrap(handle).spawn(1); }"#,
        &[r#"`.spawn(..)` on `wrap (handle)`"#],
    ),
    (
        r#"fn f() { wrap(&self.runtime).spawn(1); }"#,
        &[r#"`.spawn(..)` on `wrap (& self . runtime)`"#],
    ),
    (
        r#"fn f() { make().handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `make () . handle`"#],
    ),
    (
        r#"fn f() { self.pool.handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `self . pool . handle`"#],
    ),
    (r#"fn f() { self.handles[0].spawn(1); }"#, &[]),
    (
        r#"fn f(handles: &[Handle]) { handles[0].spawn(1); }"#,
        &[r#"`.spawn(..)` on `handles [0]`"#],
    ),
    (
        r#"fn f(x: Option<u8>, handle: Handle) { if let Some(handle) = x { drop(handle); } else { handle.spawn(1); } }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f(x: Option<u8>) { let h = Handle::current(); if let Some(h) = x { drop(h) } else { h.spawn(1); } }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"impl X { type H = Handle; }"#,
        &[r#"`type H`, an alias of a spawner"#],
    ),
    (
        r#"impl X { const R: Option<Runtime> = None; }"#,
        &[r#"`const R` of a spawner type"#],
    ),
    (
        r#"trait T { type H = Handle; }"#,
        &[r#"`type H`, an alias of a spawner"#],
    ),
    (
        r#"trait T { const C: Option<Handle>; }"#,
        &[r#"`const C` of a spawner type"#],
    ),
    (
        r#"struct S<H = Handle>(H);"#,
        &[r#"`H`, a generic defaulting to a spawner"#],
    ),
    (
        r#"thread_local! { static RT: Runtime = make(); }"#,
        &[r#"`thread_local!` of a spawner type"#],
    ),
    (
        r#"fn f() { smol::spawn(async {}).detach(); }"#,
        &[r#"`smol::spawn`"#],
    ),
    (
        r#"fn f() { let t = TaskTracker::new(); t.spawn(async {}); }"#,
        &[r#"`.spawn(..)` on `t`"#],
    ),
    (
        r#"fn f() { async_std::task::spawn(async {}); }"#,
        &[r#"`async_std::task::spawn`"#],
    ),
    (
        r#"fn f(p: P) { p.spawn_with_handle(x); }"#,
        &[r#"`.spawn_with_handle(..)` on `p`"#],
    ),
    (
        r#"fn f(p: P) { drop(futures::executor::block_on_stream(s)); }"#,
        &[r#"`futures::executor::block_on_stream`"#],
    ),
    (
        r#"fn f(p: P) { p.spawn_obj(x); }"#,
        &[r#"`.spawn_obj(..)` on `p`"#],
    ),
    (
        r#"fn f(p: P) { p.spawn_local_obj(x); }"#,
        &[r#"`.spawn_local_obj(..)` on `p`"#],
    ),
    (
        r#"fn f(x: Option<Handle>) { if let h @ Some(_) = x { h.unwrap().spawn(1); } }"#,
        &[r#"`.spawn(..)` on `h . unwrap ()`"#],
    ),
    (
        r#"fn f() { let runtime = Runtime::new().unwrap(); let h = runtime.handle().clone(); h.spawn(1); }"#,
        &[r#"`.spawn(..)` on `h`"#],
    ),
    (
        r#"fn f() { let h = tokio::runtime::Handle::current(); let g = h; g.spawn(1); }"#,
        &[r#"`.spawn(..)` on `g`"#],
    ),
    (
        r#"fn f() { let mut set = JoinSet::new(); set.spawn(1); set.spawn(2); }"#,
        &[r#"`.spawn(..)` on `set`"#, r#"`.spawn(..)` on `set`"#],
    ),
    (
        r#"fn f(rt: &tokio::runtime::Runtime) { rt.spawn(1); }"#,
        &[r#"`.spawn(..)` on `rt`"#],
    ),
    (
        r#"fn f(handle: &Handle) { tokio::task::block_in_place(|| ()); }"#,
        &[r#"`tokio::task::block_in_place`"#],
    ),
    (
        r#"fn f() { let thread = std::thread::current(); thread.unpark(); }"#,
        &[],
    ),
    (
        r#"fn f() { drop(format!("{:?}", handle.spawn(1))); }"#,
        &[r#"`spawn` in a macro"#],
    ),
    (
        r#"fn f() { drop(format!("{:?}", self.handle.spawn(1))); }"#,
        &[r#"`spawn` in a macro"#],
    ),
    (
        r#"fn f() { drop(format!("{:?}", runtime.block_on(x))); }"#,
        &[r#"`block_on` in a macro"#],
    ),
    (
        r#"fn f() { let c = std::process::Command::new("x"); drop(format!("{:?}", c.spawn())); }"#,
        &[],
    ),
    // A declared type never hides a spawner name: an alias the reader
    // cannot see (`type H = Handle` elsewhere) stays a spawner by name.
    (
        r#"fn f() { let handle: H = make(); handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f(handle: H) { handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f(runtime: &mut Rt) { runtime.enter(); }"#,
        &[r#"`.enter(..)` on `runtime`"#],
    ),
    (
        r#"fn f(builder: B) { builder.spawn(|| ()); }"#,
        &[r#"`.spawn(..)` on `builder`"#],
    ),
    (
        r#"fn f(x: X) { let handle: crate::rt::H = x.into(); handle.spawn(1); }"#,
        &[r#"`.spawn(..)` on `handle`"#],
    ),
    (
        r#"fn f() { let thread = Worker::new(); thread.spawn(1); }"#,
        &[r#"`.spawn(..)` on `thread`"#],
    ),
];

/// The forms the merged check flagged that hold no spawner: a process's
/// `Command` named `runtime` or `builder`, and another type's `builder()`.
const MERGED_FALSE_POSITIVES: &[&str] = &[
    r#"fn f() { Client::builder().spawn(); }"#,
    r#"fn f(runtime: &str) { std::process::Command::new(runtime).spawn().unwrap(); }"#,
    r#"fn f() { let mut builder = std::process::Command::new("x"); builder.spawn().unwrap(); }"#,
    r#"fn f(builder: &mut std::process::Command) { builder.spawn().unwrap(); }"#,
];

/// What the reader finds in `source`, and whether the merged check flags it.
fn read(source: &str) -> (Vec<String>, bool) {
    let file = syn::parse_file(source).unwrap_or_else(|error| panic!("{source}: {error}"));
    let files = [("x.rs".to_string(), file)];
    let found = spawn_sites(&files)
        .into_iter()
        .map(|(_, _, what)| what)
        .collect();
    (found, !master_sites(&files).is_empty())
}

/// The reader sees every form, however it is reached, and says what it is.
#[test]
fn every_spawn_form_is_seen() {
    assert!(CASES.len() > 150, "the corpus is read: {}", CASES.len());
    for (source, expected) in CASES {
        let (found, _) = read(source);
        assert_eq!(found, *expected, "{source}");
    }
}

/// Every form the merged check caught, the reader catches, but its known
/// false positives, which the reader leaves (and the merged check still
/// flags, so the list stays true).
#[test]
fn the_reader_catches_all_the_merged_check_caught() {
    for source in CASES
        .iter()
        .map(|(source, _)| *source)
        .chain(MERGED_FALSE_POSITIVES.iter().copied())
    {
        let (found, merged) = read(source);
        match (merged, MERGED_FALSE_POSITIVES.contains(&source)) {
            (true, false) => assert!(!found.is_empty(), "the merged check caught it: {source}"),
            (true, true) => assert!(
                found.is_empty(),
                "a merged false positive: {source}: {found:?}"
            ),
            (false, true) => panic!("the merged check does not flag it: {source}"),
            (false, false) => {}
        }
    }
}
