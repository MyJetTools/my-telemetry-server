fn main() {
    // Source lives in css/ - public/assets/app.css is generated and must never be
    // edited by hand.
    ci_utils::css::CssCompiler::new("./css")
        .add_file("01-tokens.css")
        .add_file("02-layout.css")
        .add_file("03-dialog.css")
        .compile("./public/assets/app.css");
}
