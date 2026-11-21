cmake_minimum_required(VERSION 3.20)
project(native_oidc_release_consumer LANGUAGES CXX)

if(NOT DEFINED NATIVE_OIDC_PACKAGE)
    message(FATAL_ERROR "NATIVE_OIDC_PACKAGE must point to the extracted Windows release package")
endif()

add_executable(native_oidc_release_consumer main.cpp)
# C++20: C++/WinRT uses standard coroutines; under C++17 it falls back to
# <experimental/coroutine>, which current MSVC rejects.
target_compile_features(native_oidc_release_consumer PRIVATE cxx_std_20)
target_include_directories(native_oidc_release_consumer PRIVATE "${NATIVE_OIDC_PACKAGE}/include")
# windowsapp provides the Windows Runtime JSON parser used to check the
# refreshed state (C++/WinRT headers ship with the Windows SDK).
target_link_libraries(native_oidc_release_consumer PRIVATE
    "${NATIVE_OIDC_PACKAGE}/native_oidc.dll.lib"
    windowsapp)
add_custom_command(TARGET native_oidc_release_consumer POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${NATIVE_OIDC_PACKAGE}/native_oidc.dll"
        "$<TARGET_FILE_DIR:native_oidc_release_consumer>/native_oidc.dll")
