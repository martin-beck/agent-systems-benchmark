Fix `parse_line` so it removes one terminal carriage return used by CRLF input,
while preserving every other leading or trailing character. Do not change the
public function name or the supplied behavior examples.
