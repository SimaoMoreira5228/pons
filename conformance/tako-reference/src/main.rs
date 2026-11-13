use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let document = tako_reference::router().into_document().map_err(|errors| {
        std::io::Error::other(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })?;
    let mut output = document.to_canonical_json()?;
    output.push('\n');
    std::io::stdout().write_all(output.as_bytes())?;
    Ok(())
}
