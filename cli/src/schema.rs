//! `schema --json`: the JSON Schema of every answer; `skill/schema.json` is this
//! output, checked by the `schema_json_is_current` test.

use clap::Args;
use freespace_core::queries::types::schema_document;

use crate::output;

#[derive(Args)]
pub struct SchemaArgs {
    /// Accepted for symmetry; the schema is always JSON
    #[arg(long)]
    pub json: bool,
}

pub fn run(_a: &SchemaArgs) -> output::Result<()> {
    output::print_json(&schema_document())?;
    Ok(())
}
