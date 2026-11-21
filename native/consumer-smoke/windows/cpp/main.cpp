#include <cstdio>
#include <string>
#include <string_view>

#include <winrt/Windows.Data.Json.h>

#include "native_oidc_windows.h"

namespace {

// Parses the refreshed state with the Windows JSON parser and checks the
// rotated tokens under token_response and a numeric top-level expires_at.
bool is_expected_refreshed_state(const char* value) {
    using winrt::Windows::Data::Json::JsonObject;
    using winrt::Windows::Data::Json::JsonValueType;
    try {
        JsonObject state{nullptr};
        if (!JsonObject::TryParse(winrt::to_hstring(std::string_view(value)), state)) {
            return false;
        }
        const JsonObject token_response = state.GetNamedObject(L"token_response");
        return token_response.GetNamedString(L"access_token") == L"new-access" &&
               token_response.GetNamedString(L"refresh_token") == L"new-refresh" &&
               state.GetNamedValue(L"expires_at").ValueType() == JsonValueType::Number;
    } catch (const winrt::hresult_error&) {
        // A missing member or a member of the wrong type.
        return false;
    }
}

// Refreshes and then revokes against the local mock provider at base_url,
// reusing one cancellation handle for both successful operations and
// freeing it afterwards.
bool refresh_and_revoke(const std::string& base_url) {
    NativeOidcWindowsCancellationHandle* cancellation =
        native_oidc_windows_cancellation_new();
    if (cancellation == nullptr) {
        std::fprintf(stderr, "cancellation handle was not created\n");
        return false;
    }

    const std::string endpoints =
        "\"authorization_endpoint\":\"" + base_url + "/authorize\","
        "\"token_endpoint\":\"" + base_url + "/token\","
        "\"client_id\":\"consumer\"";
    const std::string refresh_request =
        "{" + endpoints +
        ",\"state\":{\"token_response\":{\"access_token\":\"old\","
        "\"token_type\":\"Bearer\",\"refresh_token\":\"old-refresh\"}}}";
    const std::string revoke_request =
        "{" + endpoints + ",\"revocation_endpoint\":\"" + base_url +
        "/revoke\",\"token\":\"new-refresh\","
        "\"token_type_hint\":\"refresh_token\"}";

    bool succeeded = true;

    NativeOidcWindowsRefreshResult* refresh =
        native_oidc_windows_refresh(refresh_request.c_str(), cancellation);
    if (refresh == nullptr) {
        std::fprintf(stderr, "refresh returned no result\n");
        succeeded = false;
    } else {
        const char* value = native_oidc_windows_authorization_result_value(refresh);
        const bool refreshed =
            native_oidc_windows_authorization_result_status(refresh) ==
                NATIVE_OIDC_WINDOWS_AUTHORIZATION_SUCCESS &&
            value != nullptr && is_expected_refreshed_state(value);
        if (!refreshed) {
            std::fprintf(stderr, "unexpected refresh result: %s\n",
                         value == nullptr ? "(null)" : value);
            succeeded = false;
        }
        native_oidc_windows_authorization_result_free(refresh);
    }

    NativeOidcWindowsRevocationResult* revoke =
        native_oidc_windows_revoke(revoke_request.c_str(), cancellation);
    if (revoke == nullptr) {
        std::fprintf(stderr, "revoke returned no result\n");
        succeeded = false;
    } else {
        const char* value = native_oidc_windows_authorization_result_value(revoke);
        const bool revoked =
            native_oidc_windows_authorization_result_status(revoke) ==
                NATIVE_OIDC_WINDOWS_AUTHORIZATION_SUCCESS &&
            value != nullptr && value[0] == '\0';
        if (!revoked) {
            std::fprintf(stderr, "unexpected revoke result: %s\n",
                         value == nullptr ? "(null)" : value);
            succeeded = false;
        }
        native_oidc_windows_authorization_result_free(revoke);
    }

    native_oidc_windows_cancellation_free(cancellation);
    return succeeded;
}

}  // namespace

int main(int argc, char** argv) {
    if (argc != 2) {
        std::fprintf(stderr, "usage: native_oidc_release_consumer <mock provider base URL>\n");
        return 2;
    }
    if (!refresh_and_revoke(argv[1])) {
        return 1;
    }

    NativeOidcWindowsCancellationHandle* cancellation =
        native_oidc_windows_cancellation_new();
    if (cancellation == nullptr) {
        return 1;
    }

    native_oidc_windows_cancellation_cancel(cancellation);

    // Invalid JSON must produce an error result for every operation,
    // regardless of the cancellation state.
    const char* invalid_request = "{\"client_id\":\"consumer\",}";

    NativeOidcWindowsRefreshResult* refresh =
        native_oidc_windows_refresh(invalid_request, cancellation);
    if (refresh == nullptr ||
        native_oidc_windows_authorization_result_status(refresh) !=
            NATIVE_OIDC_WINDOWS_AUTHORIZATION_ERROR) {
        return 1;
    }
    native_oidc_windows_authorization_result_free(refresh);

    NativeOidcWindowsEndSessionResult* end_session =
        native_oidc_windows_end_session(invalid_request, cancellation);
    if (end_session == nullptr ||
        native_oidc_windows_authorization_result_status(end_session) !=
            NATIVE_OIDC_WINDOWS_AUTHORIZATION_ERROR) {
        return 1;
    }
    native_oidc_windows_authorization_result_free(end_session);

    native_oidc_windows_cancellation_free(cancellation);
    return 0;
}
