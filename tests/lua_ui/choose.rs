//! `grofe.choose(items, fn, {title?, preview?})`.

use crate::helpers::{
    DOWN, ESC, STORE_ARG, assert_api, attach, bind, press, r_stays_unset, server, wait_r,
};
use crate::support::{WAIT, lua};

fn bind_choose(srv: &crate::common::TestServer, items: &str, opts: &str) {
    bind(
        srv,
        "j",
        &format!("grofe.choose({items}, function(v) {STORE_ARG} end{opts})"),
    );
}

#[test]
fn enter_picks_the_first_item() {
    let srv = server();
    bind_choose(&srv, "{'alpha','beta','gamma'}", "");
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "j");
    a.wait_for("beta");
    a.send("\r");
    assert_eq!(wait_r(&srv), "[alpha]");
    a.wait_until("chooser closed", WAIT, |t| !t.contains("gamma"));
}

#[test]
fn down_moves_the_selection() {
    let srv = server();
    bind_choose(&srv, "{'alpha','beta','gamma'}", "");
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "j");
    a.wait_for("gamma");
    a.send(DOWN);
    a.send("\r");
    assert_eq!(wait_r(&srv), "[beta]");
}

#[test]
fn typing_filters_then_enter_picks() {
    let srv = server();
    bind_choose(&srv, "{'alpha','beta','gamma'}", "");
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "j");
    a.wait_for("gamma");
    a.send("gam");
    a.wait_until("filtered", WAIT, |t| {
        t.contains("gamma") && !t.contains("alpha")
    });
    a.send("\r");
    assert_eq!(wait_r(&srv), "[gamma]");
}

#[test]
fn cancel_gives_nil() {
    let srv = server();
    bind_choose(&srv, "{'alpha','beta'}", "");
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "j");
    a.wait_for("alpha");
    a.send(ESC);
    assert_eq!(wait_r(&srv), "NIL");
    a.wait_until("chooser closed", WAIT, |t| !t.contains("beta"));
}

#[test]
fn enter_with_no_match_does_not_call_back() {
    let srv = server();
    bind_choose(&srv, "{'alpha','beta'}", "");
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "j");
    a.wait_for("alpha");
    a.send("zzzz");
    a.wait_until("nothing matches", WAIT, |t| !t.contains("alpha"));
    a.send("\r");
    r_stays_unset(&srv);
}

/// Choice made here: an empty list calls `fn(nil)` at once, without a chooser.
#[test]
fn empty_list_calls_back_with_nil() {
    let srv = server();
    bind_choose(&srv, "{}", "");
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "j");
    assert_eq!(wait_r(&srv), "NIL");
}

#[test]
fn title_is_shown() {
    let srv = server();
    bind_choose(&srv, "{'alpha'}", ", {title = 'Pick an issue'}");
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "j");
    a.wait_for("Pick an issue");
}

#[test]
fn unicode_items_show_and_filter() {
    let srv = server();
    bind_choose(&srv, "{'日本語の課題', 'café au lait', 'Ünïcode'}", "");
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "j");
    a.wait_for("日本語の課題");
    a.wait_for("café au lait");
    a.send("café");
    a.wait_until("filtered", WAIT, |t| !t.contains("Ünïcode"));
    a.send("\r");
    assert_eq!(wait_r(&srv), "[café au lait]");
}

/// Choice made here: for a table item `fn` gets its `value`; `label` is shown.
#[test]
fn table_items_show_label_and_return_value() {
    let srv = server();
    bind_choose(
        &srv,
        "{{label = 'Shown One', value = 'v-1'}, {label = 'Shown Two', value = 'v-2'}}",
        "",
    );
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "j");
    a.wait_for("Shown Two");
    a.send(DOWN);
    a.send("\r");
    assert_eq!(wait_r(&srv), "[v-2]");
}

/// Choice made here: `preview` is a function from the item to text, shown
/// for the selected row.
#[test]
fn preview_shows_text_for_the_selected_item() {
    let srv = server();
    bind_choose(
        &srv,
        "{'alpha','beta'}",
        ", {preview = function(item) return 'PV<' .. item .. '>' end}",
    );
    let mut a = attach(&srv, 120, 30);
    press(&mut a, "j");
    a.wait_for("PV<alpha>");
    a.send(DOWN);
    a.wait_for("PV<beta>");
}

#[test]
fn bad_arguments_are_errors() {
    let srv = server();
    assert_api(&srv, "choose");
    let e = lua(&srv, "grofe.choose('nope', function() end)").expect_err("items not a list");
    assert!(e.contains("choose"), "{e}");
    let e = lua(&srv, "grofe.choose({'a'}, 'nope')").expect_err("fn not a function");
    assert!(e.contains("choose"), "{e}");
    let e = lua(&srv, "grofe.choose({'a', 5}, function() end)").expect_err("item not a string");
    assert!(e.contains("choose"), "{e}");
    assert_eq!(
        srv.cmd(&["display-message", "-p", "up"])
            .map(|s| s.trim().to_string()),
        Ok("up".into())
    );
}
