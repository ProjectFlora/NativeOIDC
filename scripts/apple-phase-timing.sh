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

phase_record() {
    local name=$1 status=$2 seconds=$3
    echo "[timing] $name: $(phase_format_seconds "$seconds") ($status)" >&2
    if [[ -n "${APPLE_PHASE_TIMING_FILE:-}" ]]; then
        printf '%s\t%s\t%s\n' "$name" "$status" "$seconds" >> "$APPLE_PHASE_TIMING_FILE"
    elif [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
        echo "- $name: $(phase_format_seconds "$seconds") ($status)" >> "$GITHUB_STEP_SUMMARY"
    fi
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
    done < "$APPLE_PHASE_TIMING_FILE"
    printf '%s\n' "$table"
    if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
        printf '%s\n\n' "$table" >> "$GITHUB_STEP_SUMMARY"
    fi
}
