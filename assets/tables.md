| Id | Name | Qty |
|----|------|-----|
| 1 | apple | 10 |
| 2 | pear | 200 |

| Id | Rule | Enforced by | Check |
|----|------|-------------|-------|
| A1a | Every order has exactly one invoice address, either postal or electronic, never both at once | `SetPostalAddress` clears `email_invoice` in the same transaction | both fields set |
| A1b | A shipped order has a tracking number; an order without one is still being packed and not shipped | decided in review; the shipper assigns the number on dispatch | shipped without number |
| A2b | Prices are in cents | `normalize_price_to_cents` rewrites decimal input when an order is imported | fractional cents, ≥ 2015 |

| a | b | c |
|---|---|---|
| xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx | yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy | zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz |

| Status | Description | Notes |
|--------|-------------|-------|
| ok | The quick brown fox jumps over the lazy dog and keeps running through the whole forest | short |
| ok | Pack my box with five dozen liquor jugs, then carry them over the bridge to the other side of town | also short |
| fail | How vexingly quick daft zebras jump | A much longer note that explains in great detail why this row failed and what to do about it next |

| Left | Right |
|------|-------|
| Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore | Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo |
| Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur | Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim |

| abcdefgh | abcdefgh | abcdefgh | abcdefgh | abcdefgh | abcdefgh | abcdefgh | abcdefgh |
|---|---|---|---|---|---|---|---|
| abcdefgh | abcdefgh | abcdefgh | abcdefgh | abcdefgh | abcdefgh | abcdefgh | abcdefgh |

| # | key | value | type | default | description | since | notes |
|---|-----|-------|------|---------|-------------|-------|-------|
| 1 | max_image_height | 30 | int | 30 | The maximum image height as terminal row count, kept proportional to the aspect ratio | 0.1 | capped at viewport width |
| 2 | hide_urls | true | bool | true | Hide link URLs and only show the link text | 0.2 | |
| 3 | table_symbols | ╭╮╰╯─│┬┴├┤┼ | string | rounded | The 11 table border characters | 0.9 | experimental |

| a | b | c |
|---|---|---|
| xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx | yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy | zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz |

| a | b |
|---|---|
| xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx | 界界界界界界界界界界 |

| a | ❤️ | 👨‍👩‍👧 |
|---|----|------|
| xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx | yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy | 🇸🇪🇩🇪🇯🇵 flags and families wrap |

| Feature | Details |
|---------|---------|
| **bold** and *italic* | A [very long link text that has to wrap somewhere along the way](https://example.com/a/very/long/url/that/should/not/count/towards/width/when/urls/are/hidden) followed by `inline code that is also rather long` and more text |
| ~~strike~~ | Plain text, plain text, plain text, plain text, plain text, plain text, plain text, plain text |

| Short | Long |
|-------|------|
| ok | Supercalifragilisticexpialidocious_Supercalifragilisticexpialidocious_Supercalifragilisticexpialidocious |

| Only |
|------|
| Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua, ut enim ad minim veniam |

| a | b | c |
|---|---|---|
|   | Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt | |
| only one cell, but quite a long one to force a shrink somewhere along the line | | x |

| Left | Center | Right |
|:-----|:------:|------:|
| Lorem ipsum dolor sit amet, consectetur adipiscing elit | short | 1 |
| sed do eiusmod tempor | Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris | 22222222 |
