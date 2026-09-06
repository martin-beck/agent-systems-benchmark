Migrate the pinned offline `textwrap` dependency from 0.15.2 to 0.16.1 and
replace the removed `fill` call with the 0.16 iterator API while preserving
newline-joined output. Use only versions listed in `vendor/INDEX`.
