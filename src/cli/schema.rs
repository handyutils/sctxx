//! `sctxx schema` — print one of the published JSON Schemas.
//!
//! The schemas are contracts, so the binary that produces an artifact is also
//! the authority on its shape.

use super::{GlobalArgs, out};
use crate::error::{Error, Result};
use clap::Args;

/// `sctxx schema`
#[derive(Debug, Args)]
pub struct SchemaArgs {
    /// Which schema: handoff, state, ops, or ir.
    name: String,
}

pub fn run(args: &SchemaArgs, _global: &GlobalArgs) -> Result<i32> {
    let name = args.name.trim().to_lowercase();
    let name = name.trim_end_matches(".v1").trim_end_matches(".json");
    match crate::SCHEMAS
        .iter()
        .find(|(candidate, _)| *candidate == name)
    {
        Some((_, body)) => {
            out(body);
            Ok(0)
        }
        None => Err(Error::Usage(format!(
            "unknown schema `{}` (expected {})",
            args.name,
            crate::SCHEMAS
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<&str>>()
                .join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_name_resolves_and_unknown_ones_are_usage_errors() {
        let global = GlobalArgs {
            quiet: true,
            ..GlobalArgs::default()
        };
        for (name, _) in crate::SCHEMAS {
            assert_eq!(
                run(
                    &SchemaArgs {
                        name: (*name).to_string()
                    },
                    &global
                )
                .expect(name),
                0
            );
        }
        // Suffixes an agent might guess still work.
        assert_eq!(
            run(
                &SchemaArgs {
                    name: "ops.v1".into()
                },
                &global
            )
            .expect("ops.v1"),
            0
        );
        assert_eq!(
            run(
                &SchemaArgs {
                    name: "nope".into()
                },
                &global
            )
            .expect_err("reject")
            .exit_code(),
            2
        );
    }
}
