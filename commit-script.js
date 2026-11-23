const { execSync } = require('child_process');

execSync('git add .', { cwd: C:/Users/saito/orca/workspaces/NativeOIDC/auto-issue-run-9-20261123T0906 });

const commitMsg = "refactor(#157): shared callback parsing, ID token validation pre-processing, token-request form, and parameter name uniqueness\n\nRemove copy-pasted security logic to single definitions:\n\n- src/parameters.rs: new module with are_parameter_names_unique()\n- src/authorization/validation.rs: add parse_callback_query() and CallbackQueryError\n- src/id_token.rs: add validate_id_token_if_present()\n- src/authorization/http.rs: add serialize_token_form()\n- src/ffi.rs: use parameters module for FFI validation\n- src/windows/data.rs: use parameters module for Windows validation\n\nNo API changes. All tests unchanged.\n\nSee plan comment at issue #157.";

execSync(`git commit -m "${commitMsg}"`, { cwd: C:/Users/saito/orca/workspaces/NativeOIDC/auto-issue-run-9-20261123T0906 });
console.log('Commit created');
