# Windows C++ user guide

Return to the [documentation index](README.md). This guide covers the
Windows-specific setup and the C++ calls. Behavior shared by every language
(state lifecycle, refresh semantics, logout policy, error categories) is
described once in the [shared user guide](user-guide.md) and is not repeated
here. The exact C ABI contract (functions, statuses, request fields, field
validation) is in the [Windows ABI reference](windows-api.md), and the JSON
request examples for each operation are in the
[Windows adapter README](../native/windows/README.md). The
[API coverage matrix](api-coverage.md) lists every operation.

The guide targets the published **v0.8.0** release. The Windows header, C
ABI, and request/response JSON on current `main` are unchanged from v0.8.0.
Check the matching tag when you install another version.

Every `cpp` block in this guide, together with the `main.cpp` of the
[minimal application](#minimal-application), is compiled by the Windows
consumer fixture, which also runs the representative
paths against a local mock provider. See
[section 9](#9-verification-coverage-and-known-limits).

## 1. Versions, installation, and build

| Item | v0.8.0 |
| --- | --- |
| Language | C++20 for the examples (C++/WinRT JSON); the header itself is C and C++ |
| Architecture | x64 (`x86_64-pc-windows-msvc`). Windows on Arm runs an x64 host under emulation; there is no Arm64 DLL |
| Toolchain | MSVC with a Windows 10/11 SDK (the examples use C++/WinRT from the SDK) |
| Windows versions | Microsoft-supported Windows client versions, reviewed each release (see the [support matrix](support-matrix.md)) |
| Runtime dependencies | None beyond Windows; the DLL contains its HTTP client and async runtime |

### Download and verify

Download `native-oidc-windows-0.8.0.zip` and `SHA256SUMS` from the
[v0.8.0 release](https://github.com/ProjectFlora/NativeOIDC/releases/tag/v0.8.0),
then verify the checksum from the download directory:

```powershell
$asset = 'native-oidc-windows-0.8.0.zip'
$expected = ((Select-String SHA256SUMS -Pattern "  $([regex]::Escape($asset))$").Line -split '\s+')[0]
$actual = (Get-FileHash $asset -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw 'SHA-256 mismatch' }
Expand-Archive $asset -DestinationPath third_party/native-oidc-windows-0.8.0
```

The ZIP root contains:

| File | Use |
| --- | --- |
| `include/native_oidc_windows.h` | C/C++ declarations; add `include/` to the include path |
| `native_oidc.dll.lib` | Import library; link it |
| `native_oidc.dll` | Runtime library; ship it next to the executable |
| `dotnet/` | .NET P/Invoke wrapper (not used from C++) |
| `README.md` | The [Windows adapter README](../native/windows/README.md) of the release |
| `LICENSE`, `THIRD_PARTY_LICENSES.txt`, `RUST_STANDARD_LIBRARY_COPYRIGHT.html` | Ship these notices with the application (see [third-party licenses](../README.md#third-party-licenses)) |

### CMake

Describe the package as an imported library. The examples also use the
Windows Runtime JSON parser (`windowsapp`) and DPAPI (`crypt32`):

```cmake
cmake_minimum_required(VERSION 3.20)
project(my_app LANGUAGES CXX)

if(NOT MSVC OR NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
    message(FATAL_ERROR "NativeOIDC for Windows requires an x64 MSVC build")
endif()

set(NATIVE_OIDC_PACKAGE "${CMAKE_CURRENT_SOURCE_DIR}/third_party/native-oidc-windows-0.8.0"
    CACHE PATH "Extracted native-oidc-windows ZIP")

add_library(native_oidc SHARED IMPORTED)
set_target_properties(native_oidc PROPERTIES
    IMPORTED_IMPLIB "${NATIVE_OIDC_PACKAGE}/native_oidc.dll.lib"
    IMPORTED_LOCATION "${NATIVE_OIDC_PACKAGE}/native_oidc.dll"
    INTERFACE_INCLUDE_DIRECTORIES "${NATIVE_OIDC_PACKAGE}/include")

add_executable(my_app main.cpp)
target_compile_features(my_app PRIVATE cxx_std_20)
target_compile_options(my_app PRIVATE /utf-8)
target_link_libraries(my_app PRIVATE native_oidc windowsapp crypt32)
add_custom_command(TARGET my_app POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "$<TARGET_FILE:native_oidc>" "$<TARGET_FILE_DIR:my_app>")
```

The project has two source files next to `CMakeLists.txt`: `auth_client.hpp`,
which holds every `cpp` block of this guide in order, and the `main.cpp` of
the [minimal application](#minimal-application). Configure for x64, build,
and run:

```powershell
cmake -S . -B build -A x64
cmake --build build --config Release
.\build\Release\my_app.exe
```

C++20 is needed by C++/WinRT, not by NativeOIDC: under C++17, C++/WinRT
falls back to `<experimental/coroutine>`, which current MSVC rejects. A
host that uses another JSON library can compile the header as C++17 or C.
Without CMake, add `include/` to the include path, link
`native_oidc.dll.lib`, and copy `native_oidc.dll` beside the executable.

### Includes

The examples in this guide use these headers:

```cpp
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <wincrypt.h>
#include <dpapi.h>

#include <chrono>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <fstream>
#include <functional>
#include <future>
#include <iterator>
#include <memory>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <system_error>
#include <thread>
#include <utility>
#include <vector>

#include <winrt/Windows.Data.Json.h>
#include <winrt/Windows.Foundation.Collections.h>  // JsonArray::Append, JsonObject::HasKey

#include "native_oidc_windows.h"

using winrt::Windows::Data::Json::JsonArray;
using winrt::Windows::Data::Json::JsonObject;
using winrt::Windows::Data::Json::JsonValue;
using winrt::Windows::Data::Json::JsonValueType;
```

## 2. Configuration, strings, and the loopback redirect

### Provider configuration

Register a native public client with the provider (no client secret) and
the loopback redirect URI, as described in the
[shared guide](user-guide.md#configure-a-native-public-client). Each
operation takes its configuration in its own JSON request, so the host keeps
the settings and builds a request per call. This structure holds them; an
empty string means "omit the field":

```cpp
struct Parameter {
    std::string name;   // UTF-8
    std::string value;  // UTF-8
};

// Provider and client settings, all UTF-8. Store them with the state: the
// state JSON does not record which provider or client it belongs to.
struct ProviderConfig {
    // Discovery: set issuer and leave both endpoints empty.
    // Explicit configuration: set both endpoints. For OIDC ID Token
    // validation also set issuer and jwks_uri together; with explicit
    // endpoints, an issuer without jwks_uri is rejected.
    std::string issuer;
    std::string authorization_endpoint;
    std::string token_endpoint;
    std::string jwks_uri;
    // Optional operation endpoints. With explicit configuration they are
    // required for end-session and revocation; with discovery they override
    // the discovered values.
    std::string end_session_endpoint;
    std::string revocation_endpoint;

    std::string client_id;
    // Portless http://127.0.0.1 templates; the adapter picks the port.
    std::string redirect_uri;
    std::string post_logout_redirect_uri;  // empty: no post-logout callback
    std::vector<std::string> scopes;
    std::vector<Parameter> additional_parameters;        // authorization request
    std::vector<Parameter> additional_token_parameters;  // code exchange
};
```

For example, discovery from an issuer:

```cpp
ProviderConfig discovered_config() {
    ProviderConfig config;
    config.issuer = "https://issuer.example";
    config.client_id = "native-client";
    config.redirect_uri = "http://127.0.0.1/oauth/callback";
    config.post_logout_redirect_uri = "http://127.0.0.1/oauth/logout";
    config.scopes = {"openid", "profile", "offline_access"};
    return config;
}
```

or explicit endpoints with OIDC validation, for a provider without a
discovery document:

```cpp
ProviderConfig explicit_config() {
    ProviderConfig config;
    config.issuer = "https://issuer.example";
    config.authorization_endpoint = "https://issuer.example/authorize";
    config.token_endpoint = "https://issuer.example/token";
    config.jwks_uri = "https://issuer.example/jwks";
    config.end_session_endpoint = "https://issuer.example/endsession";
    config.revocation_endpoint = "https://issuer.example/revoke";
    config.client_id = "native-client";
    config.redirect_uri = "http://127.0.0.1/oauth/callback";
    config.scopes = {"openid", "profile", "offline_access"};
    return config;
}
```

Endpoint URL rules (HTTPS, and where loopback HTTP is allowed) are listed in
the [ABI reference](windows-api.md#request-and-response-fields). Discovery
runs inside each operation that uses `issuer`; there is no standalone
discovery call and no way to inspect the resolved endpoints.

### UTF-8 strings and JSON

Every request is a NUL-terminated UTF-8 JSON string, and every result value
is UTF-8. Keep `std::string` values in UTF-8, and convert from UTF-16
(`std::wstring`, Win32 `W` APIs) explicitly, for example with
`winrt::to_string`. Do not pass strings in the ANSI code page. Compile with
`/utf-8` so that string literals are UTF-8 too.

Build requests with a JSON library instead of concatenating strings, so that
values are escaped correctly. The examples use the Windows Runtime parser
`Windows.Data.Json` through C++/WinRT, which ships with the Windows SDK; any
UTF-8 JSON library works the same way:

```cpp
void set_string(const JsonObject& object, std::wstring_view name, std::string_view utf8) {
    object.SetNamedValue(name, JsonValue::CreateStringValue(winrt::to_hstring(utf8)));
}

void set_optional(const JsonObject& object, std::wstring_view name, const std::string& utf8) {
    if (!utf8.empty()) {
        set_string(object, name, utf8);
    }
}

JsonArray parameters_json(const std::vector<Parameter>& parameters) {
    JsonArray array;
    for (const Parameter& parameter : parameters) {
        JsonObject item;
        set_string(item, L"name", parameter.name);
        set_string(item, L"value", parameter.value);
        array.Append(item);
    }
    return array;
}

// Fields that select the configuration, shared by every operation.
JsonObject configuration_fields(const ProviderConfig& config) {
    JsonObject request;
    set_optional(request, L"issuer", config.issuer);
    set_optional(request, L"authorization_endpoint", config.authorization_endpoint);
    set_optional(request, L"token_endpoint", config.token_endpoint);
    set_string(request, L"client_id", config.client_id);
    return request;
}

std::string to_utf8(const JsonObject& object) {
    return winrt::to_string(object.Stringify());
}
```

The authentication state is passed back to refresh as stored, without
re-serializing it. `Windows.Data.Json` stores numbers as `double`, so
parsing and re-serializing the state could change large integer values in
preserved token-response fields. This helper adds the raw state as the last
member of an object serialized by `Stringify`:

```cpp
std::string with_raw_member(std::string object_json, std::string_view name,
                            std::string_view raw_json) {
    // object_json comes from Stringify, so it ends with '}'; name needs no escaping.
    object_json.pop_back();
    if (object_json.size() > 1) {
        object_json += ',';
    }
    object_json += '"';
    object_json += name;
    object_json += "\":";
    object_json += raw_json;
    object_json += '}';
    return object_json;
}
```

### Requests for each operation

```cpp
std::string authorization_request(const ProviderConfig& config) {
    const JsonObject request = configuration_fields(config);
    set_optional(request, L"jwks_uri", config.jwks_uri);
    set_string(request, L"redirect_uri", config.redirect_uri);
    JsonArray scopes;
    for (const std::string& scope : config.scopes) {
        scopes.Append(JsonValue::CreateStringValue(winrt::to_hstring(scope)));
    }
    request.SetNamedValue(L"scopes", scopes);
    request.SetNamedValue(L"additional_parameters", parameters_json(config.additional_parameters));
    request.SetNamedValue(L"additional_token_parameters",
                          parameters_json(config.additional_token_parameters));
    return to_utf8(request);
}

std::string refresh_request(const ProviderConfig& config, std::string_view state_json) {
    const JsonObject request = configuration_fields(config);
    set_optional(request, L"jwks_uri", config.jwks_uri);
    return with_raw_member(to_utf8(request), "state", state_json);
}

std::string end_session_request(const ProviderConfig& config,
                                const std::optional<std::string>& id_token_hint) {
    const JsonObject request = configuration_fields(config);
    set_optional(request, L"end_session_endpoint", config.end_session_endpoint);
    if (id_token_hint) {
        set_string(request, L"id_token_hint", *id_token_hint);
    }
    set_optional(request, L"post_logout_redirect_uri", config.post_logout_redirect_uri);
    return to_utf8(request);
}

std::string revocation_request(const ProviderConfig& config, std::string_view token,
                               std::string_view token_type_hint) {
    const JsonObject request = configuration_fields(config);
    set_optional(request, L"revocation_endpoint", config.revocation_endpoint);
    set_string(request, L"token", token);
    set_string(request, L"token_type_hint", token_type_hint);
    return to_utf8(request);
}
```

The refresh request also accepts an `additional_parameters` array for the
refresh token request; the examples do not use it.

### Loopback redirect

`redirect_uri` and `post_logout_redirect_uri` must be portless
`http://127.0.0.1/...` URIs. For each operation the adapter binds an
available port on `127.0.0.1`, puts that port in the URI it sends, and waits
for the browser to return to it. Register the redirect with the provider in
the form it accepts for loopback clients, usually the portless URI, as
[RFC 8252 section 7.3](https://www.rfc-editor.org/rfc/rfc8252#section-7.3)
describes. `localhost`, `[::1]`, a fixed port, HTTPS, and custom schemes are
rejected. The listener's behavior is specified in
[loopback callback](loopback-callback.md).

## 3. Authorization, tokens, and expiry

### Results, handles, and ownership

The operations block until they finish. Each takes a request and a
cancellation handle and returns a result that the host owns:

- The request string only needs to live until the call returns.
- Create a cancellation handle per operation. A cancelled handle stays
  cancelled, so do not reuse one for the next operation. The handle may be
  cancelled from any thread. Free it only after the call that uses it has
  returned. A null handle makes the call return an error result.
- `native_oidc_windows_authorization_result_value` returns a pointer owned by
  the result. Copy the value, then free the result with
  `native_oidc_windows_authorization_result_free`. The same accessors and free
  function apply to the refresh, end-session, and revocation results.

These wrappers make the ownership automatic:

```cpp
struct CancellationDeleter {
    void operator()(NativeOidcWindowsCancellationHandle* handle) const noexcept {
        native_oidc_windows_cancellation_free(handle);
    }
};
using CancellationPtr = std::unique_ptr<NativeOidcWindowsCancellationHandle, CancellationDeleter>;

struct ResultDeleter {
    void operator()(NativeOidcWindowsAuthorizationResult* result) const noexcept {
        native_oidc_windows_authorization_result_free(result);
    }
};
using ResultPtr = std::unique_ptr<NativeOidcWindowsAuthorizationResult, ResultDeleter>;

enum class OperationStatus { Success, Error, Cancelled };

struct OperationResult {
    OperationStatus status = OperationStatus::Error;
    // Success: state JSON, end-session response JSON, or empty.
    // Error and Cancelled: diagnostic text, not a stable format.
    std::string value;
};

// Copies the status and value, then frees the result.
OperationResult take_result(NativeOidcWindowsAuthorizationResult* raw) {
    const ResultPtr result(raw);
    if (!result) {
        return {OperationStatus::Error, "the operation returned no result"};
    }
    const char* value = native_oidc_windows_authorization_result_value(result.get());
    std::string copy = value != nullptr ? value : "";
    switch (native_oidc_windows_authorization_result_status(result.get())) {
    case NATIVE_OIDC_WINDOWS_AUTHORIZATION_SUCCESS:
        return {OperationStatus::Success, std::move(copy)};
    case NATIVE_OIDC_WINDOWS_AUTHORIZATION_CANCELLED:
        return {OperationStatus::Cancelled, std::move(copy)};
    default:
        return {OperationStatus::Error, std::move(copy)};
    }
}
```

### Running an operation on a worker thread

Never call an operation on a UI thread: authorization waits for the user in
the browser, and the others wait on the network. `OperationRun` starts one
operation on its own thread with a fresh handle, can cancel it, and waits for
it to return before it frees the handle. It also connects an optional
`std::stop_token` to the handle **before** the call starts, so a stop
requested at any time, even before the operation began, cancels it:

```cpp
using NativeOperation = NativeOidcWindowsAuthorizationResult* (*)(
    const char* request_json_utf8, const NativeOidcWindowsCancellationHandle* cancellation);

struct CancelOnStop {
    const NativeOidcWindowsCancellationHandle* handle;
    void operator()() const noexcept {
        native_oidc_windows_cancellation_cancel(handle);
    }
};

class OperationRun {
public:
    OperationRun(NativeOperation operation, std::string request_json_utf8,
                 std::stop_token stop = {})
        : cancellation_(native_oidc_windows_cancellation_new()) {
        if (!cancellation_) {
            throw std::runtime_error("could not create a cancellation handle");
        }
        // Runs CancelOnStop right away if a stop was already requested, so
        // the call then returns the cancelled status before doing any work.
        stop_callback_.emplace(std::move(stop), CancelOnStop{cancellation_.get()});
        result_ = promise_.get_future();
        worker_ = std::thread([this, operation, request = std::move(request_json_utf8)] {
            try {
                promise_.set_value(take_result(operation(request.c_str(), cancellation_.get())));
            } catch (...) {
                promise_.set_exception(std::current_exception());
            }
        });
    }
    OperationRun(const OperationRun&) = delete;
    OperationRun& operator=(const OperationRun&) = delete;

    // Cancels a call that is still running and waits for it to return, so
    // the handle is never freed while the DLL uses it.
    ~OperationRun() {
        if (worker_.joinable()) {
            cancel();
            worker_.join();
        }
    }

    // Thread-safe; has no effect after the call returned.
    void cancel() const noexcept {
        native_oidc_windows_cancellation_cancel(cancellation_.get());
    }

    // Returns the result, or nothing if the call is still running after timeout.
    std::optional<OperationResult> wait_for(std::chrono::milliseconds timeout) {
        if (result_.wait_for(timeout) != std::future_status::ready) {
            return std::nullopt;
        }
        return wait();
    }

    // Waits for the call to return. Call wait or a successful wait_for once.
    OperationResult wait() {
        OperationResult result = result_.get();
        worker_.join();
        return result;
    }

private:
    CancellationPtr cancellation_;  // destroyed last, after the worker joined
    std::optional<std::stop_callback<CancelOnStop>> stop_callback_;
    std::promise<OperationResult> promise_;
    std::future<OperationResult> result_;
    std::thread worker_;
};
```

A GUI can hold an `OperationRun`, poll `wait_for` with a zero timeout from a
timer, and call `cancel` from a Cancel button. The examples below use it
through the `AuthClient` class, which blocks its caller and takes a
`std::stop_token` instead.

### The client: state, operations, and serialization

`AuthClient` holds the configuration and the protected store
([section 4](#4-secure-persistence-and-restoration)). It serializes every
operation that reads or writes the state, as the
[shared guide](user-guide.md#save-restore-and-refresh) requires, so a late
refresh cannot write tokens back after sign-out. Its methods block, so call
them from a background thread. To cancel one from another thread, pass a
`std::stop_token` and request a stop on its source:

```cpp
class AuthClient {
public:
    AuthClient(ProviderConfig config, std::filesystem::path state_file);

    // A stop requested on `stop` cancels the operation, whether it is
    // already running or still waiting for another operation to finish.
    std::optional<std::string> stored_state();
    OperationResult sign_in(std::chrono::milliseconds timeout, std::stop_token stop = {});
    OperationResult refresh(std::chrono::milliseconds timeout, std::stop_token stop = {});
    OperationResult end_provider_session(std::chrono::milliseconds timeout,
                                         std::stop_token stop = {});
    OperationResult sign_out(std::chrono::milliseconds timeout, std::stop_token stop = {});

private:
    OperationResult run(NativeOperation operation, std::string request,
                        std::chrono::milliseconds timeout, std::stop_token stop);
    // Protected storage, section 4. Called with state_mutex_ held.
    std::optional<std::string> load_state() const;
    void save_state(std::string_view state_json) const;
    void delete_state() const;

    ProviderConfig config_;
    std::filesystem::path state_file_;
    std::mutex state_mutex_;  // held for each whole operation
};
```

`run` applies the host's timeout. When the timeout elapses it cancels the
call and still waits for it to return. The call may have finished first, so
the result after a timeout can be any status. Callers handle it the same
way, for example by saving a successful state:

```cpp
OperationResult AuthClient::run(NativeOperation operation, std::string request,
                                std::chrono::milliseconds timeout, std::stop_token stop) {
    OperationRun run(operation, std::move(request), std::move(stop));
    if (std::optional<OperationResult> result = run.wait_for(timeout)) {
        return std::move(*result);
    }
    run.cancel();
    return run.wait();
}
```

### Authorization

`native_oidc_windows_authorize` resolves the configuration, binds the
loopback listener, opens the system browser with the authorization URL
(PKCE, state and, for `openid`, nonce are generated inside), validates the
callback, exchanges the code, validates any ID Token, and returns the
authentication-state JSON. The authorization URL and nonce are not exposed.

The call waits for the browser callback **without a timeout of its own**: if
the user closes the browser, it waits until it is cancelled. Always pass a
host timeout (or a stop from a Cancel button). Cancellation stops the wait
but cannot close the browser window.

```cpp
OperationResult AuthClient::sign_in(std::chrono::milliseconds timeout, std::stop_token stop) {
    const std::lock_guard lock(state_mutex_);
    OperationResult result =
        run(native_oidc_windows_authorize, authorization_request(config_), timeout, std::move(stop));
    if (result.status == OperationStatus::Success) {
        save_state(result.value);
    }
    return result;
}
```

From a UI, start sign-in on a background thread and marshal the result back
to the UI thread (for example with `PostMessage`). `std::jthread` passes its
own stop token. Destroying the returned thread (or calling `request_stop`)
cancels the sign-in, even one that has not started its native call yet, and
the destructor then waits for it. Destroy it before the client:

```cpp
// Runs sign-in on a background thread; on_done runs on that thread.
std::jthread start_sign_in(AuthClient& client,
                           std::function<void(const OperationResult&)> on_done) {
    return std::jthread([&client, on_done = std::move(on_done)](std::stop_token stop) {
        OperationResult result;
        try {
            result = client.sign_in(std::chrono::minutes(5), std::move(stop));
        } catch (const std::exception& error) {
            result = {OperationStatus::Error, error.what()};  // for example, storage failed
        }
        on_done(result);
    });
}
```

### Tokens and expiry

The successful value is the complete authentication state. Its format is
described in the
[ABI reference](windows-api.md#request-and-response-fields): `token_response`
holds the tokens, `expires_at` is the absolute access-token expiry in Unix
seconds (or `null`), and `oidc_nonce` is kept for later ID Token validation.
The ABI has no token or expiry accessors, so the host reads the JSON:

```cpp
struct Tokens {
    std::string access_token;
    std::string token_type;
    std::optional<std::string> refresh_token;
    std::optional<std::string> id_token;
    std::optional<std::int64_t> expires_at;  // Unix seconds
};

std::optional<std::string> optional_string(const JsonObject& object, std::wstring_view name) {
    if (!object.HasKey(name)) {
        return std::nullopt;
    }
    const auto value = object.GetNamedValue(name);
    if (value.ValueType() != JsonValueType::String) {
        return std::nullopt;  // null when absent
    }
    return winrt::to_string(value.GetString());
}

// Throws winrt::hresult_error when the JSON is not a valid state.
Tokens read_tokens(std::string_view state_json) {
    const JsonObject state = JsonObject::Parse(winrt::to_hstring(state_json));
    const JsonObject response = state.GetNamedObject(L"token_response");
    Tokens tokens;
    tokens.access_token = winrt::to_string(response.GetNamedString(L"access_token"));
    tokens.token_type = winrt::to_string(response.GetNamedString(L"token_type"));
    tokens.refresh_token = optional_string(response, L"refresh_token");
    tokens.id_token = optional_string(response, L"id_token");
    if (state.HasKey(L"expires_at") &&
        state.GetNamedValue(L"expires_at").ValueType() == JsonValueType::Number) {
        tokens.expires_at = static_cast<std::int64_t>(state.GetNamedNumber(L"expires_at"));
    }
    return tokens;
}

std::int64_t unix_now() {
    return std::chrono::duration_cast<std::chrono::seconds>(
               std::chrono::system_clock::now().time_since_epoch())
        .count();
}

// True when the access token expires within skew_seconds. An unknown expiry
// returns false: use the token and refresh when the resource server rejects it.
bool needs_refresh(const Tokens& tokens, std::int64_t now, std::int64_t skew_seconds = 60) {
    return tokens.expires_at && *tokens.expires_at <= now + skew_seconds;
}
```

Use the access token with its `token_type` for resource-server calls. The ID
Token is an identity assertion, not an API credential. Do not log tokens or
the state JSON.

## 4. Secure persistence and restoration

The state contains the access, refresh, and ID Tokens, so the host must
protect it. This example stores it in a file encrypted with DPAPI for the
current Windows user. The file is replaced atomically, so a crash does not
leave a half-written state:

```cpp
struct LocalFreeDeleter {
    void operator()(void* memory) const noexcept {
        LocalFree(memory);
    }
};

[[noreturn]] void throw_last_error(const char* operation) {
    throw std::system_error(static_cast<int>(GetLastError()), std::system_category(), operation);
}

// Application-specific DPAPI entropy. It ties the data to this application
// but is not a secret: any process running as the same user can read it.
DATA_BLOB state_entropy() {
    static char entropy[] = "com.example.my-app/native-oidc-state/v1";
    return {static_cast<DWORD>(sizeof(entropy) - 1), reinterpret_cast<BYTE*>(entropy)};
}

class ProtectedStateStore {
public:
    explicit ProtectedStateStore(std::filesystem::path file) : file_(std::move(file)) {}

    void save(std::string_view state_json) const {
        DATA_BLOB plaintext{static_cast<DWORD>(state_json.size()),
                            reinterpret_cast<BYTE*>(const_cast<char*>(state_json.data()))};
        DATA_BLOB entropy = state_entropy();
        DATA_BLOB encrypted{};
        if (!CryptProtectData(&plaintext, L"NativeOIDC authentication state", &entropy, nullptr,
                              nullptr, CRYPTPROTECT_UI_FORBIDDEN, &encrypted)) {
            throw_last_error("CryptProtectData");
        }
        const std::unique_ptr<BYTE, LocalFreeDeleter> owned(encrypted.pbData);

        if (file_.has_parent_path()) {
            std::filesystem::create_directories(file_.parent_path());
        }
        const std::filesystem::path temporary = temporary_file();
        try {
            {
                std::ofstream out(temporary, std::ios::binary | std::ios::trunc);
                out.write(reinterpret_cast<const char*>(encrypted.pbData),
                          static_cast<std::streamsize>(encrypted.cbData));
                out.close();
                if (!out) {
                    throw std::runtime_error("could not write the protected state");
                }
            }
            if (!MoveFileExW(temporary.c_str(), file_.c_str(),
                             MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
                throw_last_error("MoveFileExW");
            }
        } catch (...) {
            // Do not leave a copy of the tokens behind when the save fails.
            std::error_code ignored;
            std::filesystem::remove(temporary, ignored);
            throw;
        }
    }

    // Returns nothing when no state is stored.
    std::optional<std::string> load() const {
        std::ifstream in(file_, std::ios::binary);
        if (!in) {
            if (!std::filesystem::exists(file_)) {
                return std::nullopt;
            }
            throw std::runtime_error("could not read the protected state");
        }
        std::vector<char> bytes{std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>()};
        DATA_BLOB encrypted{static_cast<DWORD>(bytes.size()), reinterpret_cast<BYTE*>(bytes.data())};
        DATA_BLOB entropy = state_entropy();
        DATA_BLOB plaintext{};
        if (!CryptUnprotectData(&encrypted, nullptr, &entropy, nullptr, nullptr,
                                CRYPTPROTECT_UI_FORBIDDEN, &plaintext)) {
            throw_last_error("CryptUnprotectData");
        }
        const std::unique_ptr<BYTE, LocalFreeDeleter> owned(plaintext.pbData);
        std::string state(reinterpret_cast<const char*>(plaintext.pbData), plaintext.cbData);
        SecureZeroMemory(plaintext.pbData, plaintext.cbData);
        return state;
    }

    // Also removes a temporary file left by an interrupted save, which can
    // hold a newer refresh token than the stored state.
    // Attempts both deletions, then reports the first failure.
    void remove() const {
        std::error_code first_error;
        for (const std::filesystem::path& path : {file_, temporary_file()}) {
            std::error_code error;
            std::filesystem::remove(path, error);  // a missing file is not an error
            if (error && !first_error) {
                first_error = error;
            }
        }
        if (first_error) {
            throw std::system_error(first_error, "could not delete the protected state");
        }
    }

private:
    std::filesystem::path temporary_file() const {
        std::filesystem::path temporary = file_;
        temporary += L".tmp";
        return temporary;
    }

    std::filesystem::path file_;
};

AuthClient::AuthClient(ProviderConfig config, std::filesystem::path state_file)
    : config_(std::move(config)), state_file_(std::move(state_file)) {}

std::optional<std::string> AuthClient::load_state() const {
    return ProtectedStateStore(state_file_).load();
}

void AuthClient::save_state(std::string_view state_json) const {
    ProtectedStateStore(state_file_).save(state_json);
}

void AuthClient::delete_state() const {
    ProtectedStateStore(state_file_).remove();
}

std::optional<std::string> AuthClient::stored_state() {
    const std::lock_guard lock(state_mutex_);
    return load_state();
}
```

What the host still decides:

- **Location:** a per-user directory such as one under
  `%LOCALAPPDATA%` (`SHGetKnownFolderPath(FOLDERID_LocalAppData)`), one file
  per provider and client ID, because the state does not record its
  configuration.
- **Threat model:** DPAPI current-user protection stops other users and
  offline copies of the file, not malware running as the same user. Windows
  Credential Manager is an alternative, but its blob limit
  (`CRED_MAX_CREDENTIAL_BLOB_SIZE`, 2560 bytes) can be smaller than a state
  with an ID Token.
- **Plaintext copies:** `SecureZeroMemory` clears the DPAPI buffer, but
  `std::string` copies are not cleared. Keep the decrypted state short-lived.

Restoring is reading the file: there is no separate restore call. Pass the
loaded JSON to refresh unchanged. Restoration does not contact the provider,
so a restored token may already have been revoked.

## 5. Refresh and re-save

Refresh sends the stored refresh token and returns a replacement state.
Save it after every success, because the provider may rotate the refresh
token. On error or cancellation, keep the stored state: it is still the
latest one the host has. See the
[shared guide](user-guide.md#save-restore-and-refresh) for what that does and
does not guarantee.

```cpp
OperationResult AuthClient::refresh(std::chrono::milliseconds timeout, std::stop_token stop) {
    const std::lock_guard lock(state_mutex_);
    const std::optional<std::string> state = load_state();
    if (!state) {
        return {OperationStatus::Error, "not signed in"};
    }
    OperationResult result =
        run(native_oidc_windows_refresh, refresh_request(config_, *state), timeout, std::move(stop));
    if (result.status == OperationStatus::Success) {
        save_state(result.value);
    }
    return result;
}
```

The library never refreshes on its own. A typical host refreshes shortly
before expiry, then uses the new access token:

```cpp
// Diagnostics: provider descriptions are untrusted text. Never log tokens.
void log_diagnostic(std::string_view message) {
    OutputDebugStringW(winrt::to_hstring(message).c_str());
    OutputDebugStringW(L"\n");
}

// Returns an access token to call the resource server, or nothing when the
// user needs to sign in (again) or the refresh did not finish.
std::optional<std::string> usable_access_token(AuthClient& client) {
    const std::optional<std::string> state = client.stored_state();
    if (!state) {
        return std::nullopt;  // not signed in
    }
    Tokens tokens = read_tokens(*state);
    if (needs_refresh(tokens, unix_now())) {
        const OperationResult refreshed = client.refresh(std::chrono::seconds(30));
        switch (refreshed.status) {
        case OperationStatus::Success:
            tokens = read_tokens(refreshed.value);
            break;
        case OperationStatus::Cancelled:
            return std::nullopt;  // timeout or user; the stored state is unchanged
        case OperationStatus::Error:
            // Rejected (for example invalid_grant), network, or configuration.
            // The stored state is unchanged; retry later or sign in again.
            log_diagnostic(refreshed.value);
            return std::nullopt;
        }
    }
    return tokens.access_token;
}
```

A refresh response without a new refresh token, scope, or ID Token keeps the
previous value in the returned state, so always save the returned state as a
whole.

## 6. End-session, revocation, and local sign-out

These are three separate actions, as the
[shared guide](user-guide.md#logout-revocation-and-local-deletion) explains.

### Provider end-session (RP-Initiated Logout)

`native_oidc_windows_end_session` opens the provider's logout page in the
system browser. Read the ID Token for `id_token_hint` before deleting the
state:

```cpp
OperationResult AuthClient::end_provider_session(std::chrono::milliseconds timeout,
                                                 std::stop_token stop) {
    const std::lock_guard lock(state_mutex_);
    std::optional<std::string> id_token_hint;
    if (const std::optional<std::string> state = load_state()) {
        id_token_hint = read_tokens(*state).id_token;
    }
    return run(native_oidc_windows_end_session, end_session_request(config_, id_token_hint),
               timeout, std::move(stop));
}
```

- Without `post_logout_redirect_uri`, success means only that the browser
  was opened, with an empty value. It does not confirm the provider's logout.
- With a `post_logout_redirect_uri` registered at the provider, the call
  waits on the loopback listener and validates the returned `state`. The
  success value is JSON with `state` and `additional_parameters`. Like
  authorization, the wait has no timeout of its own.
- End-session does not touch the stored state.

### Token revocation and local deletion

`native_oidc_windows_revoke` asks the provider to invalidate one token.
Sign-out revokes the refresh token (or the access token when there is none),
then deletes the stored state **whatever the revocation result**: local
sign-out must not depend on the provider:

```cpp
OperationResult AuthClient::sign_out(std::chrono::milliseconds timeout, std::stop_token stop) {
    const std::lock_guard lock(state_mutex_);
    OperationResult revocation{OperationStatus::Success, ""};
    try {
        if (const std::optional<std::string> state = load_state()) {
            const Tokens tokens = read_tokens(*state);
            const bool has_refresh_token = tokens.refresh_token.has_value();
            revocation = run(native_oidc_windows_revoke,
                             revocation_request(config_,
                                                has_refresh_token ? *tokens.refresh_token
                                                                  : tokens.access_token,
                                                has_refresh_token ? "refresh_token" : "access_token"),
                             timeout, std::move(stop));
        }
    } catch (const std::exception& error) {
        revocation = {OperationStatus::Error, error.what()};
    } catch (const winrt::hresult_error& error) {
        revocation = {OperationStatus::Error, winrt::to_string(error.message())};
    }
    delete_state();
    return revocation;  // informational: the user is signed out locally either way
}
```

A successful revocation has an empty value. HTTP 200 counts as success even
for an already-invalid token. With explicit configuration, revocation needs
`revocation_endpoint`; with discovery, the provider must advertise one.
Without it, the call returns an error and sign-out still deletes the state.
A host that also wants the access token revoked makes a second call.

For a full logout, call `end_provider_session` first (it needs the ID
Token), then `sign_out`. Because `AuthClient` holds its state lock for each
whole operation, a refresh that started earlier finishes before sign-out
deletes the state, and cannot write it back afterwards.

## 7. Dynamic registration and additional parameters

The Windows ABI has **no dynamic client registration**. Register the client
with the provider beforehand, or register it with another component and put
the resulting `client_id` in `ProviderConfig`. The shared core's
registration API is not exposed through the C ABI.

Additional parameters are exposed:

- `ProviderConfig::additional_parameters` goes into the authorization request
  (for example `prompt` or `login_hint`). Reserved names (`client_id`,
  `redirect_uri`, `response_type`, `scope`, `state`, `nonce`,
  `code_challenge`, `code_challenge_method`) are rejected.
- `ProviderConfig::additional_token_parameters` goes into the code exchange.
  Reserved token-request names, such as `client_secret`, `code`,
  `code_verifier`, and `grant_type`, are rejected.
- Refresh accepts its own `additional_parameters` array (not used by these
  examples).

Names must be unique in each array. A rejected parameter returns an error
result before the browser opens.

## 8. Cancellation, errors, and lifecycle

### Cancellation and timeouts

- Every operation takes a cancellation handle; `OperationRun` gives each one
  its own and connects it to a `std::stop_token` before the call starts, so
  a stop is never lost to a race with the operation's start.
  `native_oidc_windows_cancellation_cancel` is thread-safe and idempotent.
- Cancellation makes the blocking call return with the cancelled status as
  soon as the current step notices it. It cannot undo a request the
  provider already processed, and it cannot close the browser.
- The host owns every timeout. Authorization and end-session with a
  post-logout redirect wait for the browser indefinitely. Choose a timeout
  that leaves time to sign in (the example uses five minutes).
- After cancelling, always wait for the call to return before freeing the
  handle or exiting. `OperationRun`'s destructor does this.

### Errors

The result status is the only structured error information. The value of an
error or cancelled result is diagnostic text for logs and support; do not
parse it or show provider descriptions as trusted text. The
[ABI reference](windows-api.md#c-abi) lists the statuses, and the
[shared guide](user-guide.md#cancellation-timeouts-and-errors) maps the
failure categories.

| Status | Typical causes | Host action |
| --- | --- | --- |
| Success | — | Save the returned state (authorize, refresh); continue |
| Cancelled | Host timeout, Cancel button, shutdown | Keep the stored state; offer to try again |
| Error | Invalid request or configuration, invalid redirect, rejected parameter, browser could not be opened, listener failure | A bug or a deployment problem: fix it rather than retrying |
| Error | Provider rejection (`invalid_grant` on refresh), HTTP or network failure, invalid response or ID Token | Keep the stored state; retry later with backoff, or sign in again |

The C ABI does not tell these two kinds of error apart. A configuration
error usually appears on the first call after a deployment, while the
second kind can appear at any time. Retry with a limit, and ask the user to
sign in again when refresh keeps failing. The examples'
own code adds C++ exceptions: `std::system_error` from DPAPI and file
operations, and `winrt::hresult_error` from parsing an invalid state.

### Process and lifecycle constraints

- Load the DLL through the import library for the life of the process. Do
  not unload it while an operation is running.
- Operations are independent: each binds its own loopback port and runtime.
  Still serialize the operations that change one state, as `AuthClient` does.
- The adapter cannot detect that the user closed the browser; only the host
  timeout or cancellation ends the wait.

### Minimal application

This `main.cpp` completes the CMake project from
[section 1](#cmake). `auth_client.hpp` holds the `cpp` blocks above, in
order. Because those blocks define functions, include the header from one
source file only, or split it into a header and a source file. A console
application may block its main thread; a GUI uses `start_sign_in` instead:

```cpp main.cpp
#include "auth_client.hpp"  // every cpp block of this guide, in order

#include <cstdio>

// The per-user state file, under %LOCALAPPDATA%.
std::filesystem::path state_file_path() {
    std::wstring directory(MAX_PATH, L'\0');
    const DWORD length =
        GetEnvironmentVariableW(L"LOCALAPPDATA", directory.data(), static_cast<DWORD>(directory.size()));
    if (length == 0 || length >= directory.size()) {
        throw std::runtime_error("LOCALAPPDATA is not set");
    }
    directory.resize(length);
    return std::filesystem::path(directory) / L"MyApp" / L"native-oidc-state.bin";
}

// Usage: my_app [--sign-in]
// --sign-in starts a new sign-in even when a state is stored, for example
// after the provider rejected its refresh token.
int main(int argc, char** argv) {
    const bool force_sign_in = argc > 1 && std::string_view(argv[1]) == "--sign-in";
    try {
        AuthClient client(discovered_config(), state_file_path());
        if (force_sign_in || !client.stored_state()) {
            std::puts("Opening the browser to sign in...");
            const OperationResult result = client.sign_in(std::chrono::minutes(5));
            if (result.status != OperationStatus::Success) {
                // A failed sign-in keeps any previously stored state.
                std::fprintf(stderr, "Sign-in did not complete: %s\n", result.value.c_str());
                return 1;
            }
        }
        if (!usable_access_token(client)) {
            // The stored state is kept, so a transient failure can succeed on
            // the next launch; a rejected refresh token needs a new sign-in.
            std::fputs("No usable access token. Try again later, or run with --sign-in "
                       "to sign in again.\n",
                       stderr);
            return 1;
        }
        std::puts("Signed in. Call the resource server with the access token.");
        return 0;
    } catch (const std::exception& error) {
        std::fprintf(stderr, "%s\n", error.what());
    } catch (const winrt::hresult_error& error) {
        std::fprintf(stderr, "%s\n", winrt::to_string(error.message()).c_str());
    }
    return 1;
}
```

## 9. Verification, coverage, and known limits

### What the examples are checked against

The C++ consumer fixture in
[`native/consumer-smoke/windows/cpp`](../native/consumer-smoke/windows/cpp)
extracts every `cpp` block of this guide, in order, into `auth_client.hpp`
at CMake configure time, and compiles it with
[`guide_examples.cpp`](../native/consumer-smoke/windows/cpp/guide_examples.cpp)
against the package's header, import library, and DLL, using the CMake
target shown in [section 1](#cmake). It also builds the
[minimal application](#minimal-application) from the `cpp main.cpp` block
and that header; that program is linked but not run, because it opens a
browser. The example runs against a fresh local
mock provider in its `guide` mode
([`mock-provider`](../native/consumer-smoke/windows/mock-provider/Program.cs)),
which checks every request it receives:

- DPAPI save, load (the file does not contain the token in plain text), and
  removal. A failed replace leaves no temporary file. Removal also deletes a
  temporary file left by an interrupted save, and still deletes the state
  when that temporary file cannot be deleted.
- `usable_access_token` refreshes an expired state, the provider rotates the
  refresh token, and the replacement state is saved.
- A refresh rejected with `invalid_grant`, a refresh cancelled by the host
  timeout, and a refresh cancelled through a `std::stop_token` from
  another thread each leave the stored state unchanged.
- A sign-in whose stop was requested before it started returns the
  cancelled status without opening the browser.
- `sign_out` revokes the refresh token and deletes the state, and still
  deletes it when revocation fails with HTTP 503.
- Errors that need no browser: a single explicit endpoint, a non-loopback
  redirect, a reserved additional parameter, end-session without its
  endpoint, and refresh without a stored state. Sign-in through
  `start_sign_in` also takes this path.

Run it with the other Windows consumers on a Windows host with Visual
Studio (C++ and CMake) and the .NET SDK:

```powershell
# A downloaded release ZIP (verify its checksum first, see section 1)
scripts/check-windows-release-consumer.ps1 -Package native-oidc-windows-0.8.0.zip
```

CI runs the same script on every change to the Windows adapter, its
fixtures, or this guide (`windows_core` on Windows Server, `windows_client`
on a Windows 11 client runner with x64 emulation), against a package built
from the branch. The `Published release consumer validation` workflow
checks out the release tag's own scripts, so for v0.8.0 it runs only the
consumers that existed at that tag, not these guide examples.

Evidence for this guide (commit `f65587e`):

- Executed: CI built a package from the branch, then compiled all 19 `cpp`
  blocks and passed all 26 checks against it, both on Windows Server
  ([`windows_core`](https://github.com/ProjectFlora/NativeOIDC/actions/runs/45299430987/job/138128023271))
  and on a Windows 11 client with x64 emulation
  ([`windows_client`](https://github.com/ProjectFlora/NativeOIDC/actions/runs/45299430987/job/138129767357)).
  The Windows module, header, and ABI are unchanged between v0.8.0 and that
  commit.
- Compiled and linked only: MSVC 19.52 (`/std:c++20 /W4 /utf-8`, no
  warnings) against the published v0.8.0 header and `native_oidc.dll.lib`
  (ZIP SHA-256 verified against `SHA256SUMS`).
- **Not run:** the examples against the published v0.8.0 `native_oidc.dll`.
  The local machine's Smart App Control blocks unsigned executables, and the
  release workflow does not run branch scripts (see above).

### Coverage by operation

| Operation | Section | Availability | Checked by |
| --- | --- | --- | --- |
| Install, link, run (x64) | [1](#1-versions-installation-and-build) | Available | Fixture build and run |
| Discovery / explicit / OIDC configuration | [2](#provider-configuration) | Available in operation JSON; no standalone discovery or inspection | Explicit: fixture; discovery: compile only |
| Authorization (browser, loopback, exchange) | [3](#authorization) | Available | Compile and early errors only; browser flow by the manual [real-browser verification](real-browser-verification.md) (through .NET) |
| Tokens and expiry | [3](#tokens-and-expiry) | Host reads state JSON | Fixture |
| Secure persistence and restoration | [4](#4-secure-persistence-and-restoration) | Host (DPAPI example) | Fixture |
| Refresh and re-save | [5](#5-refresh-and-re-save) | Available | Fixture: success, rejection, cancellation |
| End-session | [6](#provider-end-session-rp-initiated-logout) | Available | Compile and early error only; browser flow by Windows unit tests with a simulated browser |
| Revocation and local deletion | [6](#token-revocation-and-local-deletion) | Revocation available; deletion is host | Fixture: success and failure |
| Dynamic registration | [7](#7-dynamic-registration-and-additional-parameters) | Not exposed | — |
| Additional parameters | [7](#7-dynamic-registration-and-additional-parameters) | Available | Fixture: rejection |
| Cancellation and timeouts | [8](#cancellation-and-timeouts) | Available; timeouts are host | Fixture |
| Structured errors | [8](#errors) | Not exposed (status and text only) | Fixture: status branches |

### Known limits

- The browser steps of authorization and end-session are not run by the
  fixture; CI has no interactive browser. Real-provider interoperability is
  outside these checks; see
  [interoperability status](live-idp-interoperability.md).
- The DPAPI example protects against other users and offline copies only.
- Only x64 is distributed.
