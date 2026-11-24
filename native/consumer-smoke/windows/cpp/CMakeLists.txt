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
# order, into one generated header, so the guide's code is what gets built.
set(NATIVE_OIDC_GUIDE "${CMAKE_CURRENT_SOURCE_DIR}/../../../../docs/windows-cpp-guide.md"
    CACHE FILEPATH "C++ guide whose cpp blocks are compiled")
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${NATIVE_OIDC_GUIDE}")
file(READ "${NATIVE_OIDC_GUIDE}" guide_text)
string(REPLACE "\r\n" "\n" guide_text "${guide_text}")
set(guide_code "// Generated from docs/windows-cpp-guide.md by CMakeLists.txt. Do not edit.\n#pragma once\n")
set(guide_blocks 0)
while(TRUE)
    string(FIND "${guide_text}" "```cpp\n" block_start)
    if(block_start EQUAL -1)
        break()
    endif()
    math(EXPR block_start "${block_start} + 7")
    string(SUBSTRING "${guide_text}" ${block_start} -1 guide_text)
    string(FIND "${guide_text}" "\n```" block_end)
    if(block_end EQUAL -1)
        message(FATAL_ERROR "Unterminated cpp block in ${NATIVE_OIDC_GUIDE}")
    endif()
    string(SUBSTRING "${guide_text}" 0 ${block_end} block)
    string(APPEND guide_code "\n${block}\n")
    math(EXPR block_end "${block_end} + 4")
    string(SUBSTRING "${guide_text}" ${block_end} -1 guide_text)
    math(EXPR guide_blocks "${guide_blocks} + 1")
endwhile()
if(guide_blocks EQUAL 0)
    message(FATAL_ERROR "No cpp blocks found in ${NATIVE_OIDC_GUIDE}")
endif()
message(STATUS "Compiling ${guide_blocks} cpp blocks from ${NATIVE_OIDC_GUIDE}")
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/generated/native_oidc_guide.hpp" "${guide_code}")

add_executable(native_oidc_guide_examples guide_examples.cpp)
target_compile_features(native_oidc_guide_examples PRIVATE cxx_std_20)
target_compile_options(native_oidc_guide_examples PRIVATE /utf-8)
target_include_directories(native_oidc_guide_examples PRIVATE "${CMAKE_CURRENT_BINARY_DIR}/generated")
target_link_libraries(native_oidc_guide_examples PRIVATE native_oidc windowsapp crypt32)
add_custom_command(TARGET native_oidc_guide_examples POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "$<TARGET_FILE:native_oidc>"
        "$<TARGET_FILE_DIR:native_oidc_guide_examples>")
