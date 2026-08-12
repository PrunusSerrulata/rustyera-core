use erabasic_data::{UserIndexRegistration, VariableSchema};

pub(super) fn add_registrations(
    schema: &VariableSchema,
    registrations: &mut Vec<UserIndexRegistration>,
) {
    if schema.dimensions.len() == 1 {
        registrations.push(UserIndexRegistration {
            variable_name: schema.id.name().to_owned(),
            source_stem: schema.id.name().to_owned(),
            dimension: None,
            length: schema.dimensions[0],
        });
        return;
    }

    for (index, length) in schema.dimensions.iter().copied().enumerate() {
        let dimension = index + 1;
        registrations.push(UserIndexRegistration {
            variable_name: schema.id.name().to_owned(),
            source_stem: format!("{}@{dimension}", schema.id.name()),
            dimension: Some(dimension),
            length,
        });
    }
}
