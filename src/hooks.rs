//! Method names a framework calls on a subclass.
//!
//! Pure: a table and a predicate, no I/O. `analyze` consults it for a method
//! on a class with a base, where "no references" means the framework holds the
//! only one.

/// Method names called by the standard library or a common framework on an
/// instance of a subclass — the class's base is the only reference to them.
///
/// Grouped by the framework that calls them; a test keeps the list free of
/// duplicates. Linear lookup: two hundred entries per audited method is
/// nothing next to the `tyf refs` round-trip that precedes it.
const EXACT: &[&str] = &[
    // asyncio protocols and transports
    "connection_lost",
    "connection_made",
    "data_received",
    "datagram_received",
    "eof_received",
    "error_received",
    "pause_writing",
    "pipe_connection_lost",
    "pipe_data_received",
    "process_exited",
    "resume_writing",
    // cmd.Cmd
    "completedefault",
    "emptyline",
    "postcmd",
    "postloop",
    "precmd",
    "preloop",
    // json.JSONEncoder / JSONDecoder, codecs
    "decode",
    "default",
    "encode",
    "iterencode",
    "raw_decode",
    // logging.Filter / Handler / Formatter
    "acquire",
    "createLock",
    "doRollover",
    "emit",
    "filter",
    "flush",
    "format",
    "formatException",
    "formatMessage",
    "formatStack",
    "formatTime",
    "handle",
    "handleError",
    "release",
    "shouldRollover",
    "usesTime",
    // socketserver, http.server
    "address_string",
    "end_headers",
    "finish",
    "finish_request",
    "handle_error",
    "log_error",
    "log_message",
    "log_request",
    "process_request",
    "send_head",
    "server_activate",
    "server_bind",
    "service_actions",
    "setup",
    "translate_path",
    "verify_request",
    "version_string",
    // threading.Thread, multiprocessing.Process
    "run",
    // unittest.TestCase / TestResult
    "addError",
    "addFailure",
    "addSkip",
    "addSuccess",
    "runTest",
    "setUp",
    "setUpClass",
    "startTest",
    "startTestRun",
    "stopTest",
    "stopTestRun",
    "tearDown",
    "tearDownClass",
    // pytest xunit-style setup and teardown
    "setup_class",
    "setup_function",
    "setup_method",
    "setup_module",
    "teardown",
    "teardown_class",
    "teardown_function",
    "teardown_method",
    "teardown_module",
    // ast.NodeVisitor, docutils
    "generic_visit",
    "visit",
    // xml.sax handlers
    "characters",
    "endDocument",
    "endElement",
    "endElementNS",
    "endPrefixMapping",
    "ignorableWhitespace",
    "processingInstruction",
    "startDocument",
    "startElement",
    "startElementNS",
    "startPrefixMapping",
    // io.IOBase family
    "close",
    "detach",
    "fileno",
    "isatty",
    "read",
    "readable",
    "readinto",
    "readline",
    "readlines",
    "seek",
    "seekable",
    "tell",
    "truncate",
    "writable",
    "write",
    // importlib finders and loaders, pickle
    "create_module",
    "exec_module",
    "find_class",
    "find_spec",
    "get_code",
    "get_data",
    "get_filename",
    "get_source",
    "invalidate_caches",
    "is_package",
    "persistent_id",
    "persistent_load",
    "reducer_override",
    // HTTP method handlers: Django View, Flask MethodView, Tornado, aiohttp
    "delete",
    "get",
    "head",
    "options",
    "patch",
    "post",
    "put",
    "trace",
    // Django views, forms, models, admin, middleware, commands, fields
    "add_arguments",
    "authenticate",
    "contribute_to_class",
    "db_type",
    "deconstruct",
    "dispatch",
    "form_invalid",
    "form_valid",
    "formfield",
    "from_db_value",
    "full_clean",
    "get_absolute_url",
    "get_context_data",
    "get_db_prep_save",
    "get_db_prep_value",
    "get_form",
    "get_form_class",
    "get_form_kwargs",
    "get_initial",
    "get_object",
    "get_prep_value",
    "get_queryset",
    "get_success_url",
    "get_template_names",
    "get_urls",
    "get_user",
    "has_add_permission",
    "has_change_permission",
    "has_delete_permission",
    "has_permission",
    "has_view_permission",
    "process_exception",
    "process_response",
    "process_template_response",
    "process_view",
    "ready",
    "render",
    "save",
    "to_python",
    "value_to_string",
    // Django REST framework
    "create",
    "destroy",
    "filter_queryset",
    "get_permissions",
    "get_serializer",
    "get_serializer_class",
    "has_object_permission",
    "list",
    "paginate_queryset",
    "partial_update",
    "perform_create",
    "perform_destroy",
    "perform_update",
    "retrieve",
    "to_internal_value",
    "to_representation",
    "update",
    "validate",
    // Tornado RequestHandler / WebSocketHandler
    "check_origin",
    "get_current_user",
    "initialize",
    "on_connection_close",
    "on_finish",
    "open",
    "prepare",
    "set_default_headers",
    "write_error",
    // click ParamType / Command
    "convert",
    "get_command",
    "get_metavar",
    "invoke",
    "list_commands",
    "shell_complete",
    // SQLAlchemy TypeDecorator
    "process_bind_param",
    "process_literal_param",
    "process_result_value",
    "python_type",
    // pydantic
    "model_post_init",
    // Scrapy
    "close_spider",
    "closed",
    "from_crawler",
    "from_settings",
    "open_spider",
    "parse",
    "process_item",
    "start_requests",
    // Airflow / Luigi / Prefect operators and tasks
    "complete",
    "execute",
    "output",
    "poke",
    "post_execute",
    "pre_execute",
    "requires",
    // Sphinx / docutils directives and transforms
    "apply",
    "handle_signature",
    "resolve_xref",
    // Textual / Kivy apps
    "build",
    "compose",
    // Qt models and delegates
    "columnCount",
    "createEditor",
    "data",
    "eventFilter",
    "flags",
    "headerData",
    "index",
    "paint",
    "parent",
    "rowCount",
    "setData",
    "setEditorData",
    "setModelData",
    "sizeHint",
    "updateEditorGeometry",
];

/// Naming conventions a framework dispatches on: `visit_Call` (ast, docutils),
/// `do_GET` (http.server, cmd), `on_message` (Textual, discord.py, Kivy,
/// Celery), `clean_email` (Django forms), `validate_email` (DRF),
/// `handle_starttag` (html.parser), `test*` (unittest discovery), `pytest_*`
/// (plugin hooks), `watch_*` / `action_*` (Textual), `depart_*` (docutils),
/// `help_*` / `complete_*` (cmd).
const PREFIXES: &[&str] = &[
    "action_",
    "clean_",
    "complete_",
    "depart_",
    "do_",
    "handle_",
    "help_",
    "on_",
    "pytest_",
    "test",
    "validate_",
    "visit_",
    "watch_",
];

/// `keyPressEvent`, `closeEvent`, `paintEvent`: Qt's event handlers.
const SUFFIXES: &[&str] = &["Event"];

/// Whether a framework calls a method of this name on a subclass.
///
/// Takes the method's own name, not the qualname. The caller decides whether
/// the class has a base at all; without one, `run` and `filter` are ordinary
/// names and stay audited.
#[must_use]
pub fn is_framework_hook(method: &str) -> bool {
    EXACT.contains(&method)
        || PREFIXES.iter().any(|p| method.len() > p.len() && method.starts_with(p))
        || SUFFIXES.iter().any(|s| method.len() > s.len() && method.ends_with(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pytest_xunit_hooks_are_hooks() {
        for name in ["setup_method", "teardown_class", "setup_module", "teardown"] {
            assert!(is_framework_hook(name), "pytest calls `{name}` by name");
        }
    }

    #[test]
    fn stdlib_protocol_methods_are_hooks() {
        for name in
            ["filter", "emit", "setUp", "tearDownClass", "run", "default", "connection_made"]
        {
            assert!(is_framework_hook(name), "`{name}` is called by the standard library");
        }
    }

    #[test]
    fn framework_conventions_match_by_prefix_or_suffix() {
        for name in
            ["visit_Call", "do_GET", "on_message", "clean_email", "keyPressEvent", "test_it"]
        {
            assert!(is_framework_hook(name), "`{name}` follows a framework naming convention");
        }
    }

    #[test]
    fn ordinary_names_are_not_hooks() {
        // `attest` and `contest`: a prefix is a prefix, not a substring. `event`:
        // the Qt suffix is case-sensitive.
        for name in ["helper", "compute_total", "visitor", "done", "event", "attest", "contest"] {
            assert!(!is_framework_hook(name), "`{name}` is nobody's protocol");
        }
    }

    #[test]
    fn a_bare_prefix_is_not_a_hook() {
        // `on_` alone names nothing a framework would call.
        for name in ["on_", "do_", "test", "Event"] {
            assert!(!is_framework_hook(name), "`{name}` has no name after the convention");
        }
    }

    #[test]
    fn the_table_has_no_duplicates() {
        let mut sorted = EXACT.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), before, "a name listed under two frameworks is listed once");
    }
}
