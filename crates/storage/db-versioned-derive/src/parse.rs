use proc_macro2::Ident;
use std::collections::HashMap;
use syn::{
    Attribute, DataEnum, DataStruct, DeriveInput, Error, Field, Fields, Lit, Path, Result, Type,
};

/// Parsed representation of a versioned type
pub struct VersionedInput {
    pub ident: Ident,
    pub vis: syn::Visibility,
    pub current_path: Option<Path>,
    pub kind: VersionedKind,
    pub versions: Vec<String>, // List of all versions found
}

pub enum VersionedKind {
    Struct(VersionedStruct),
    Enum(VersionedEnum),
}

pub struct VersionedStruct {
    pub fields: Vec<VersionedField>,
}

pub struct VersionedEnum {
    pub variants: Vec<VersionedVariant>,
}

pub struct VersionedField {
    pub ident: Option<Ident>,
    pub vis: syn::Visibility,
    pub ty: Type,
    pub versions: HashMap<String, String>, // version -> type_path mapping
    pub added_in: Option<String>,
    pub removed_after: Option<String>,
}

pub struct VersionedVariant {
    pub ident: Ident,
    pub fields: Fields,
}

impl VersionedInput {
    pub fn from_struct(input: &DeriveInput, data: &DataStruct) -> Result<Self> {
        let current_path = parse_current_path(&input.attrs)?;
        let mut all_versions = Vec::new();

        let fields = data
            .fields
            .iter()
            .map(|f| {
                let versioned_field = VersionedField::from_field(f)?;
                // Collect all versions mentioned in field attributes
                for version in versioned_field.versions.keys() {
                    if !all_versions.contains(version) {
                        all_versions.push(version.clone());
                    }
                }
                if let Some(ref v) = versioned_field.added_in {
                    if !all_versions.contains(v) {
                        all_versions.push(v.clone());
                    }
                }
                if let Some(ref v) = versioned_field.removed_after {
                    if !all_versions.contains(v) {
                        all_versions.push(v.clone());
                    }
                }
                Ok(versioned_field)
            })
            .collect::<Result<Vec<_>>>()?;

        // Sort versions (v6, v7, v8, etc.)
        all_versions.sort_by(|a, b| {
            let a_num = a.trim_start_matches('v').parse::<u32>().unwrap_or(0);
            let b_num = b.trim_start_matches('v').parse::<u32>().unwrap_or(0);
            a_num.cmp(&b_num)
        });

        Ok(VersionedInput {
            ident: input.ident.clone(),
            vis: input.vis.clone(),
            current_path,
            kind: VersionedKind::Struct(VersionedStruct { fields }),
            versions: all_versions,
        })
    }

    pub fn from_enum(input: &DeriveInput, data: &DataEnum) -> Result<Self> {
        let current_path = parse_current_path(&input.attrs)?;

        let variants = data
            .variants
            .iter()
            .map(|v| Ok(VersionedVariant { ident: v.ident.clone(), fields: v.fields.clone() }))
            .collect::<Result<Vec<_>>>()?;

        Ok(VersionedInput {
            ident: input.ident.clone(),
            vis: input.vis.clone(),
            current_path,
            kind: VersionedKind::Enum(VersionedEnum { variants }),
            versions: Vec::new(), // Enums don't have versioned fields for now
        })
    }
}

impl VersionedField {
    fn from_field(field: &Field) -> Result<Self> {
        let mut versions = HashMap::new();
        let mut added_in = None;
        let mut removed_after = None;

        // Parse #[version(...)] attributes on the field
        for attr in &field.attrs {
            if !attr.path().is_ident("versioned") {
                continue;
            }

            attr.parse_nested_meta(|meta| {
                let ident = meta
                    .path
                    .get_ident()
                    .ok_or_else(|| Error::new_spanned(&meta.path, "expected identifier"))?
                    .to_string();

                if ident == "added_in" {
                    let value = meta.value()?;
                    let lit: Lit = value.parse()?;
                    if let Lit::Str(s) = lit {
                        added_in = Some(s.value());
                    }
                } else if ident == "removed_after" {
                    let value = meta.value()?;
                    let lit: Lit = value.parse()?;
                    if let Lit::Str(s) = lit {
                        removed_after = Some(s.value());
                    }
                } else if ident.starts_with('v') {
                    // Version-specific type mapping (e.g., v6 = "v6::ResourceBoundsMapping")
                    let value = meta.value()?;
                    let lit: Lit = value.parse()?;
                    if let Lit::Str(s) = lit {
                        versions.insert(ident, s.value());
                    }
                }

                Ok(())
            })?;
        }

        Ok(VersionedField {
            ident: field.ident.clone(),
            vis: field.vis.clone(),
            ty: field.ty.clone(),
            versions,
            added_in,
            removed_after,
        })
    }
}

fn parse_current_path(attrs: &[Attribute]) -> Result<Option<Path>> {
    for attr in attrs {
        if !attr.path().is_ident("versioned") {
            continue;
        }

        let mut current_path = None;

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("current") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(s) = lit {
                    current_path = Some(syn::parse_str::<Path>(&s.value())?);
                }
            }
            Ok(())
        })?;

        return Ok(current_path);
    }

    Ok(None)
}
