// Runs the examples of docs/windows-cpp-guide.md against the local mock
// provider in its `guide` mode. The generated header contains every cpp
// block of the guide, in order (see CMakeLists.txt), so this file only
// calls the guide's code and checks the results. The provider checks the
// requests and their order; this program checks what the host sees.
//
// The requests sent must match GuideSteps in ../mock-provider/Program.cs.
// Everything after them fails before any network request or browser launch.

#include "native_oidc_guide.hpp"

#include <cstdio>

namespace {

int failures = 0;

void check(bool condition, const char* description) {
    std::fprintf(stderr, "%s: %s\n", condition ? "ok" : "FAILED", description);
    if (!condition) {
        ++failures;
    }
}

ProviderConfig mock_config(const std::string& base_url) {
    ProviderConfig config;
    config.authorization_endpoint = base_url + "/authorize";
    config.token_endpoint = base_url + "/token";
    config.revocation_endpoint = base_url + "/revoke";
    config.client_id = "guide-client";
    config.redirect_uri = "http://127.0.0.1/oauth/callback";
    config.scopes = {"profile"};
    return config;
}

std::string stored_state_json(std::string_view refresh_token, std::int64_t expires_at) {
    return std::string(R"({"token_response":{"access_token":"guide-access-1","token_type":"Bearer",)") +
           R"("refresh_token":")" + std::string(refresh_token) + R"("},"expires_at":)" +
           std::to_string(expires_at) + "}";
}

std::string file_bytes(const std::filesystem::path& file) {
    std::ifstream in(file, std::ios::binary);
    return {std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>()};
}

void check_storage(const std::filesystem::path& file) {
    const ProtectedStateStore store(file);
    store.remove();
    check(!store.load().has_value(), "load returns nothing before the first save");
    const std::string state = stored_state_json("guide-refresh-1", 1);
    store.save(state);
    check(store.load() == state, "DPAPI save and load round-trip the state");
    const std::string bytes = file_bytes(file);
    check(!bytes.empty() && bytes.find("guide-access-1") == std::string::npos &&
              bytes.find("guide-refresh-1") == std::string::npos,
          "the stored file does not contain the tokens in plain text");
    check(!std::filesystem::exists(std::filesystem::path(file) += L".tmp"),
          "no temporary file is left after the atomic replace");
}

void check_refresh(AuthClient& client) {
    // Step 1: the stored state expired at 1970-01-01T00:00:01Z.
    check(usable_access_token(client) == std::optional<std::string>("guide-access-2"),
          "usable_access_token refreshes an expired state");
    const std::optional<std::string> refreshed = client.stored_state();
    check(refreshed.has_value(), "the refreshed state is stored");
    if (!refreshed) {
        return;
    }
    const Tokens tokens = read_tokens(*refreshed);
    check(tokens.refresh_token == std::optional<std::string>("guide-refresh-2"),
          "the rotated refresh token is saved");
    check(tokens.expires_at.has_value() && *tokens.expires_at > unix_now(),
          "the new absolute expiry is saved");
    check(!needs_refresh(tokens, unix_now()), "the refreshed token does not need a refresh");

    // Step 2: the provider rejects the refresh token.
    const OperationResult rejected = client.refresh(std::chrono::seconds(30));
    check(rejected.status == OperationStatus::Error, "a rejected refresh returns an error");
    check(client.stored_state() == refreshed, "a rejected refresh keeps the stored state");

    // Step 3: the provider never answers; the host timeout cancels.
    const OperationResult timed_out = client.refresh(std::chrono::seconds(3));
    check(timed_out.status == OperationStatus::Cancelled, "the host timeout cancels a refresh");
    check(client.stored_state() == refreshed, "a timed-out refresh keeps the stored state");

    // Step 4: the provider never answers; another thread requests a stop.
    std::stop_source stop;
    OperationResult cancelled;
    std::thread worker([&client, &cancelled, token = stop.get_token()] {
        try {
            cancelled = client.refresh(std::chrono::seconds(60), token);
        } catch (const std::exception& error) {
            cancelled = {OperationStatus::Error, error.what()};
        }
    });
    std::this_thread::sleep_for(std::chrono::seconds(3));
    stop.request_stop();
    worker.join();
    check(cancelled.status == OperationStatus::Cancelled,
          "a stop from another thread cancels a running refresh");
    check(client.stored_state() == refreshed, "a cancelled refresh keeps the stored state");
}

void check_sign_out(AuthClient& client, const std::filesystem::path& file) {
    // Step 5: revocation succeeds.
    const OperationResult revoked = client.sign_out(std::chrono::seconds(30));
    check(revoked.status == OperationStatus::Success && revoked.value.empty(),
          "sign_out revokes the refresh token");
    check(!client.stored_state().has_value(), "sign_out deletes the stored state");

    // Step 6: revocation fails with HTTP 503.
    ProtectedStateStore(file).save(stored_state_json("guide-refresh-3", unix_now() + 3600));
    const OperationResult failed = client.sign_out(std::chrono::seconds(30));
    check(failed.status == OperationStatus::Error, "a failed revocation is reported");
    check(!client.stored_state().has_value(),
          "sign_out deletes the stored state when revocation fails");
}

// None of these reach the provider or open a browser.
void check_local_errors(const std::string& base_url, const std::filesystem::path& file) {
    AuthClient signed_out(mock_config(base_url), file);
    check(signed_out.refresh(std::chrono::seconds(10)).status == OperationStatus::Error,
          "refresh without a stored state returns an error");

    ProviderConfig single_endpoint = mock_config(base_url);
    single_endpoint.token_endpoint.clear();
    AuthClient single(single_endpoint, file);
    check(single.sign_in(std::chrono::seconds(10)).status == OperationStatus::Error,
          "sign_in rejects a single explicit endpoint");

    ProviderConfig localhost = mock_config(base_url);
    localhost.redirect_uri = "http://localhost/oauth/callback";
    AuthClient wrong_redirect(localhost, file);
    check(wrong_redirect.sign_in(std::chrono::seconds(10)).status == OperationStatus::Error,
          "sign_in rejects a non-127.0.0.1 redirect");

    ProviderConfig reserved = mock_config(base_url);
    reserved.additional_parameters = {{"state", "chosen-by-host"}};
    AuthClient reserved_parameter(reserved, file);
    check(reserved_parameter.sign_in(std::chrono::seconds(10)).status == OperationStatus::Error,
          "sign_in rejects a reserved additional parameter");

    check(signed_out.end_provider_session(std::chrono::seconds(10)).status ==
              OperationStatus::Error,
          "end-session with explicit endpoints requires end_session_endpoint");

    // A valid configuration: only the stop prevents the browser launch.
    AuthClient valid(mock_config(base_url), file);
    std::stop_source stopped;
    stopped.request_stop();
    check(valid.sign_in(std::chrono::seconds(10), stopped.get_token()).status ==
              OperationStatus::Cancelled,
          "a stop requested before sign_in starts cancels it");

    std::promise<OperationResult> background;
    std::jthread thread = start_sign_in(
        single, [&background](const OperationResult& result) { background.set_value(result); });
    std::future<OperationResult> finished = background.get_future();
    check(finished.wait_for(std::chrono::seconds(10)) == std::future_status::ready &&
              finished.get().status == OperationStatus::Error,
          "start_sign_in reports the result from its thread");
    thread = {};

    check(!signed_out.stored_state().has_value(), "failed operations store no state");
}

void run(const std::string& base_url, const std::filesystem::path& file) {
    check_storage(file);
    AuthClient client(mock_config(base_url), file);
    check_refresh(client);
    check_sign_out(client, file);
    check_local_errors(base_url, file);
}

}  // namespace

int main(int argc, char** argv) {
    if (argc != 2) {
        std::fprintf(stderr, "usage: native_oidc_guide_examples <mock provider base URL>\n");
        return 2;
    }
    const std::filesystem::path directory =
        std::filesystem::temp_directory_path() /
        ("native-oidc-guide-" + std::to_string(GetCurrentProcessId()));
    try {
        run(argv[1], directory / "state.bin");
    } catch (const std::exception& error) {
        std::fprintf(stderr, "FAILED: unexpected exception: %s\n", error.what());
        ++failures;
    } catch (const winrt::hresult_error& error) {
        std::fprintf(stderr, "FAILED: unexpected exception: %s\n",
                     winrt::to_string(error.message()).c_str());
        ++failures;
    }
    std::error_code ignored;
    std::filesystem::remove_all(directory, ignored);
    return failures == 0 ? 0 : 1;
}
