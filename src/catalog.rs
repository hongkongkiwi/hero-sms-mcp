use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub name: String,
    pub operation_id: String,
    pub method: String,
    pub path: String,
    pub parameters: Vec<Value>,
    pub mutating: bool,
    pub input_schema: Value,
    #[serde(skip)]
    validator: OnceLock<jsonschema::Validator>,
}

impl Operation {
    pub fn valid(&self, arguments: &Value) -> bool {
        self.validator
            .get_or_init(|| {
                jsonschema::options()
                    .should_validate_formats(true)
                    .build(&self.input_schema)
                    .expect("embedded schema is valid")
            })
            .is_valid(arguments)
    }
}

pub fn operations() -> &'static [Operation] {
    static CATALOG: OnceLock<Vec<Operation>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../reference/catalog.json"))
            .expect("embedded catalog is valid")
    })
}

pub fn find(name: &str) -> Option<&'static Operation> {
    operations()
        .iter()
        .find(|op| op.name == name || op.operation_id == name)
}
