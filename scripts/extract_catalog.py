"""Extract API facts only. Run with python3 -I and pass the reference file path."""
import json
import pathlib
import sys

source = json.loads(pathlib.Path(sys.argv[1]).read_text())

def resolve(value):
    if isinstance(value, list):
        return [resolve(item) for item in value]
    if not isinstance(value, dict):
        return value
    if "$ref" in value:
        target = source
        for part in value["$ref"].removeprefix("#/").split("/"):
            target = target[part]
        return resolve(target)
    return {key: ({name: resolve(schema) for name, schema in item.items()} if key == "properties" else resolve(item)) for key, item in value.items()
            if key in {"type", "properties", "required", "items", "enum", "oneOf", "allOf", "anyOf", "minimum", "maximum", "minLength", "maxLength", "minItems", "maxItems", "pattern", "format", "name", "in", "style", "explode", "schema"}}

names = {
    ("get", "/activations/history"): "getActivationHistory",
    ("get", "/emails"): "getEmailActivations",
    ("post", "/emails"): "buyEmailActivation",
    ("post", "/emails/batch"): "buyEmailActivationBatch",
    ("get", "/emails/{emailId}"): "getEmailActivation",
    ("delete", "/emails/{emailId}"): "cancelEmailActivation",
    ("post", "/emails/{emailId}/reorder"): "reorderEmailActivation",
    ("get", "/emails/domains"): "getEmailDomains",
    ("get", "/emails/offers"): "getEmailOffers",
    ("get", "/stats/deliverability"): "getDeliverabilityStats",
}
mutations = {"getNumber", "getNumberV2", "setStatus", "reactivate", "prolong", "getRentNumber", "saFinishActivation", "saCancelActivation"}
operations = []
for path, methods in source["paths"].items():
    for method, operation in methods.items():
        if method not in {"get", "post", "delete", "put", "patch"}:
            continue
        op_id = operation["operationId"]
        parameters = [resolve(param) for param in operation.get("parameters", [])]
        properties = {param["name"]: param["schema"] for param in parameters}
        required = [param["name"] for param in parameters if param.get("required")]
        body = operation.get("requestBody")
        if body:
            while "$ref" in body:
                target = source
                for part in body["$ref"][2:].split("/"):
                    target = target[part]
                body = target
            properties["body"] = resolve(body["content"]["application/json"]["schema"])
            if body.get("required") or properties["body"].get("required"):
                required.append("body")
        operations.append({"name": names.get((method, path), op_id), "operationId": op_id,
                           "method": method.upper(), "path": path, "parameters": parameters,
                           "mutating": method != "get" or op_id in mutations,
                           "inputSchema": {"type": "object", "properties": properties,
                                           "required": required, "additionalProperties": False}})
pathlib.Path(sys.argv[2]).write_text(json.dumps(operations, indent=2) + "\n")
