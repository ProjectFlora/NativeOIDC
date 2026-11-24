cmake_minimum_required(VERSION 3.20)
project(native_oidc_release_consumer LANGUAGES CXX)

if(NOT DEFINED NATIVE_OIDC_PACKAGE)
    message(FATAL_ERROR "NATIVE_OIDC_PACKAGE must point to the extracted Windows release package")
endif()
if(NOT MSVC OR NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
    message(FATAL_ERROR "NativeOIDC for Windows requires an x64 MSVC build")
endif()

# The imported target from docs/windows-cpp-guide.md, section 1.
add_library(native_oidc SHARED IMPORTED)
set_target_properties(native_oidc PROPERTIES
    IMPORTED_IMPLIB "${NATIVE_OIDC_PACKAGE}/native_oidc.dll.lib"
    IMPORTED_LOCATION "${NATIVE_OIDC_PACKAGE}/native_oidc.dll"
    INTERFACE_INCLUDE_DIRECTORIES "${NATIVE_OIDC_PACKAGE}/include")

add_executable(native_oidc_release_consumer main.cpp)
# C++20: C++/WinRT uses standard coroutines; under C++17 it falls back to
# <experimental/coroutine>, which current MSVC rejects.
target_compile_features(native_oidc_release_consumer PRIVATE cxx_std_20)
# windowsapp provides the Windows Runtime JSON parser used to check the
# refreshed state (C++/WinRT headers ship with the Windows SDK).
target_link_libraries(native_oidc_release_consumer PRIVATE native_oidc windowsapp)
add_custom_command(TARGET native_oidc_release_consumer POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "$<TARGET_FILE:native_oidc>"
        "$<TARGET_FILE_DIR:native_oidc_release_consumer>")

# The C++ guide's examples. Every ```cpp block of the guide is copied, in
# order, into the generated auth_client.hpp, and the ```cpp main.cpp block
# into the generated main.cpp, so the guide's code is what gets built.
set(NATIVE_OIDC_GUIDE "${CMAKE_CURRENT_SOURCE_DIR}/../../../../docs/windows-cpp-guide.md"
    CACHE FILEPATH "C++ guide whose cpp blocks are compiled")
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${NATIVE_OIDC_GUIDE}")
file(READ "${NATIVE_OIDC_GUIDE}" guide_text)
string(REPLACE "\r\n" "\n" guide_text "${guide_text}")

# Concatenates the bodies of the blocks in the variable named text_var that
# open with `fence` (including its newline) into out_code and counts them in
# out_count. The text is passed by name so that its semicolons stay intact.
function(native_oidc_extract_blocks text_var fence out_code out_count)
    set(text "${${text_var}}")
    string(LENGTH "${fence}" fence_length)
    set(code "")
    set(count 0)
    while(TRUE)
        string(FIND "${text}" "${fence}" block_start)
        if(block_start EQUAL -1)
            break()
        endif()
        math(EXPR block_start "${block_start} + ${fence_length}")
        string(SUBSTRING "${text}" ${block_start} -1 text)
        string(FIND "${text}" "\n```" block_end)
        if(block_end EQUAL -1)
            message(FATAL_ERROR "Unterminated code block in ${NATIVE_OIDC_GUIDE}")
        endif()
        string(SUBSTRING "${text}" 0 ${block_end} block)
        string(APPEND code "\n${block}\n")
        math(EXPR block_end "${block_end} + 4")
        string(SUBSTRING "${text}" ${block_end} -1 text)
        math(EXPR count "${count} + 1")
    endwhile()
    set(${out_code} "${code}" PARENT_SCOPE)
    set(${out_count} ${count} PARENT_SCOPE)
endfunction()

set(generated_notice "// Generated from docs/windows-cpp-guide.md by CMakeLists.txt. Do not edit.\n")
native_oidc_extract_blocks(guide_text "```cpp\n" guide_code guide_blocks)
if(guide_blocks EQUAL 0)
    message(FATAL_ERROR "No cpp blocks found in ${NATIVE_OIDC_GUIDE}")
endif()
native_oidc_extract_blocks(guide_text "```cpp main.cpp\n" main_code main_blocks)
if(NOT main_blocks EQUAL 1)
    message(FATAL_ERROR "Expected one cpp main.cpp block in ${NATIVE_OIDC_GUIDE}, found ${main_blocks}")
endif()
message(STATUS "Compiling ${guide_blocks} cpp blocks and main.cpp from ${NATIVE_OIDC_GUIDE}")
set(generated_dir "${CMAKE_CURRENT_BINARY_DIR}/generated")
file(WRITE "${generated_dir}/auth_client.hpp" "${generated_notice}#pragma once\n${guide_code}")
file(WRITE "${generated_dir}/main.cpp" "${generated_notice}${main_code}")

# guide_examples runs against the mock provider. guide_app is the guide's
# minimal application; it is built and linked but not run, because it opens
# the system browser.
foreach(guide_target IN ITEMS native_oidc_guide_examples native_oidc_guide_app)
    if(guide_target STREQUAL "native_oidc_guide_examples")
        add_executable(${guide_target} guide_examples.cpp)
    else()
        add_executable(${guide_target} "${generated_dir}/main.cpp")
    endif()
    target_compile_features(${guide_target} PRIVATE cxx_std_20)
    target_compile_options(${guide_target} PRIVATE /utf-8)
    target_include_directories(${guide_target} PRIVATE "${generated_dir}")
    target_link_libraries(${guide_target} PRIVATE native_oidc windowsapp crypt32)
    add_custom_command(TARGET ${guide_target} POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
            "$<TARGET_FILE:native_oidc>"
            "$<TARGET_FILE_DIR:${guide_target}>")
endforeach()
