mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};

use support::type_check::type_checks;

#[test]
fn a_pagination_iterator_is_typed_with_the_item_the_items_pointer_names() {
    let document = support::runtime::paginated_document(serde_json::json!({
        "request": {"kind": "cursor", "field": "cursor"},
        "items": "/items",
        "next": "/next_cursor"
    }));
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    type_checks(
        &files,
        r#"
import { createClient } from "./client.js";
import type { EventsListError } from "./client.js";
import type { Event } from "./types.js";
import type { Result } from "./errors.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const all: AsyncIterable<Result<Event, EventsListError>> = client.events.listAll({ query: {} });
console.log(all);
"#,
    );
}
