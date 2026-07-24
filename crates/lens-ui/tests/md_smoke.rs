#[test]
fn safe_prefix_and_init_link() {
    let s = lens_ui::md::safe_prefix("**wor");
    assert_eq!(s, "**wor**");
}
