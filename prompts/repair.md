---
id: repair
version: 1
---

CHUNK_ID: {{chunk_id}}

Some of the operations you returned were rejected. Return corrected operations for ONLY those,
or an empty `ops` list if they cannot be fixed with the evidence you were shown. Do not repeat
the operations that were accepted.

# REJECTIONS

{{rejections}}

# REMINDERS

- Every source range must lie inside evt {{evt_start}}–{{evt_end}}, and at least one cited event
  must appear in the transcript rows you were given.
- A `constraint` needs `quote` copied verbatim from a `[user]` row in this chunk.
- `text` ≤ 60 words, `why` ≤ 40 words, `quote` ≤ 50 words.
- Only reference item ids that exist and are active.

# RESPONSE SCHEMA

{{schema}}

Return only the JSON object.
