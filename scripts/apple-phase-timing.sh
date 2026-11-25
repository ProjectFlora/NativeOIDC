# Phase timing for the Apple packaging scripts. Source it; do not run it.
#
#   phase_begin NAME   start a phase (an open phase is ended first)
#   phase_end          end the open phase as ok
#   phase_fail_open    end the open phase, if any, as failed (call from an
#                      EXIT trap so a phase that stops the script is recorded)
#   phase_record NAME STATUS SECONDS
#                      record a row that was measured another way
#   phase_write_summary TITLE
#                      print the recorded rows as a Markdown table, and append
#                      it to $GITHUB_STEP_SUMMARY when that is set
#
# Rows go to $APPLE_PHASE_TIMING_FILE (name<TAB>status<TAB>seconds) when it is
# set, so scripts run by scripts/package-apple-xcframework.sh add to one table.
# Without it, a script run on its own appends each row to the step summary as
# a list item. Every row is also logged as a `[timing]` line on stderr.
#
# The phases are explicit begin/end calls rather than a wrapper that runs a
# command: bash ignores `set -e` inside a function called from an `if`
# condition, so a wrapper would keep going after a failing command. Durations
# use bash's $SECONDS (whole seconds, available in the macOS bash 3.2).

_apple_phase_name=""
_apple_phase_started=0

phase_format_seconds() {
    local seconds=$1
    if ((seconds >= 60)); then
        printf '%dm%02ds' $((seconds / 60)) $((seconds % 60))
    else
        printf '%ds' "$seconds"
    fi
}

# Timing is diagnostic only. Every write is best-effort: a failed write prints
# a warning and returns 0, so under `set -e` it can never stop the script or
# an EXIT trap's cleanup, or change the exit status.
_phase_append() {
    local destination=$1 line=$2
    if ! { printf '%s\n' "$line" >> "$destination"; } 2>/dev/null; then
        echo "warning: could not write phase timing to $destination" >&2 || true
    fi
    return 0
}

phase_record() {
    local name=$1 status=$2 seconds=$3 formatted
    formatted=$(phase_format_seconds "$seconds")
    echo "[timing] $name: $formatted ($status)" >&2 || true
    if [[ -n "${APPLE_PHASE_TIMING_FILE:-}" ]]; then
        _phase_append "$APPLE_PHASE_TIMING_FILE" "$name"$'\t'"$status"$'\t'"$seconds"
    elif [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
        _phase_append "$GITHUB_STEP_SUMMARY" "- $name: $formatted ($status)"
    fi
    return 0
}

phase_begin() {
    if [[ -n "$_apple_phase_name" ]]; then
        phase_end
    fi
    _apple_phase_name=$1
    _apple_phase_started=$SECONDS
}

_phase_close() {
    local status=$1
    if [[ -z "$_apple_phase_name" ]]; then
        return 0
    fi
    local name=$_apple_phase_name
    _apple_phase_name=""
    phase_record "$name" "$status" $((SECONDS - _apple_phase_started))
}

phase_end() {
    _phase_close ok
}

phase_fail_open() {
    _phase_close failed
}

phase_write_summary() {
    local title=$1 table name status seconds
    if [[ -z "${APPLE_PHASE_TIMING_FILE:-}" || ! -f "$APPLE_PHASE_TIMING_FILE" ]]; then
        return 0
    fi
    table="### $title"$'\n\n'"| Phase | Result | Time |"$'\n'"|---|---|---:|"
    while IFS=$'\t' read -r name status seconds; do
        table+=$'\n'"| $name | $status | $(phase_format_seconds "$seconds") |"
    done < "$APPLE_PHASE_TIMING_FILE" || true
    printf '%s\n' "$table" || true
    if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
        _phase_append "$GITHUB_STEP_SUMMARY" "$table"$'\n'
    fi
    return 0
}
