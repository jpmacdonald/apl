use crossterm::style::Stylize;

fn main() {
    let name = "jq";
    let version = "1.8.1";
    let name_width = 16;
    let version_width = 12;

    let symbol = " ";
    let symbol_styled = symbol.dark_grey();

    let name_part = format!("{: <width$}", name, width = name_width);
    let version_part = format!("{: <width$}", version, width = version_width);

    let line = format!(
        "{}  {} {} ",
        symbol_styled,
        name_part.cyan(),
        version_part.white()
    );

    println!("012345678901234567890123456789");
    println!("   PACKAGE          VERSION");
    println!("{}", line);
}
