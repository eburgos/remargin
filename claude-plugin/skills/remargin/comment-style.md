# Comment style

How to write a comment body so a human can read it. Critical rule 9 states the requirements; this file holds the heuristic and the worked examples.

## Why this exists

Comments written as dense prose get rejected, and the rejection costs a full round. Real reactions to real agent comments:

> What are you talking about? Please specify step by step what you need

> Stop talking with references and give me WITH STRUCTURE what you need, otherwise I'm not answering

> Which other 12 routes? I am fucking tired of you not giving details

> That is not of your concern and I would REALLY appreciate much you not leaving those kind of useless caveats in your comments that will only pollute my view

None of those comments was missing information. Each one was missing *shape* — the reader could not get at what was already there.

## Choosing the container

| Content shape | Container |
| --- | --- |
| One fact | One line |
| Two to four parallel items, short values | Bullet list |
| Three or more items, each with two or more attributes | Table |
| Ordered procedure | Numbered list |
| Code, config, command output, wire payload | Fenced block with a language tag |
| Flow with branches, state machine, sequence, dependency graph | Fenced ```mermaid block |
| Screenshot or diagram file | `![alt](assets/<filename>)` in the body, with the file passed to `attachments` in the same call |
| One topic with distinct phases | `###` headings |
| Distinct topics | Separate replies (rule 12), never headings |

A horizontal rule (`---`) between major blocks of a long comment is welcome.

**Attaching alone renders nothing.** `attachments` uploads the file beside the document and lists it in the comment's `attachments` header; only the `![alt](assets/<filename>)` line in the body displays it. Use both in the same call, with the body path matching the header entry.

**Width cuts both ways.** The same three-column table was cramped in a narrow sidebar and stretched with dead space on a wide monitor. Keep cells short, and put the longest value in the last column so it has somewhere to run.

## Worked example: prose to table

A reply naming where three endpoints are implemented.

**Before** — three parallel facts flattened into one sentence, so the reader parses commas to find the boundaries:

> All three live in the API gateway: the import form's fields come from `httpSharedItemGetDocumentType` at `packages/api-gateway/src/controllers/ecm.ts:2424`, the generate form's from `packages/api-gateway/src/controllers/template.ts:832`, and the combined form's from `packages/api-gateway/src/controllers/template.ts:2038`.

**After** — same content, same word count, scannable:

> All three anonymous endpoints that serve form definitions live in the API gateway:
>
> | Form type | Route | Implemented at |
> | --- | --- | --- |
> | Import | `GET /app/shared-items/document-types/:documentTypeId` | `ecm.ts:2424` |
> | Generate | `GET /app/shared-items/template-version/:templateVersionId/info` | `template.ts:832` |
> | Combined | `GET /app/shared-items/combined/:id/definition` | `template.ts:2038` |
>
> Paths are relative to `packages/api-gateway/src/controllers/`.

The table also exposed a gap the prose hid: the route was missing, and the route is what actually identifies an endpoint. Structure surfaces omissions that prose lets you skate past.

## Worked example: trailing metadata

**Before** — the answer, then an aside about something the reader did not ask about and cannot act on:

> Posted as `zuv`, threaded under the parent. It is a self-reply, so nothing was acked.
>
> One observation I cannot act on: the sidebar shows a `to:` badge and an `unacked` chip even though no recipient was passed. If that is putting it in a queue somewhere, the recipient derivation may be coming from the parent.

**After** — the addendum is cut, because it changed nothing the reader would do:

> Posted as `zuv`, threaded under the parent. It is a self-reply, so nothing was acked.

If a caveat genuinely changes the reader's next action, it belongs inline at the point where it is relevant — not parked at the end.

## Worked example: prose to diagram

**Before** — four transitions the reader must hold in their head:

> On a write from an agent identity the gate runs the reject-tier checks first; if any fires, the write is refused with the line and the fix. If none fires the write lands and the warn-tier checks run over the same body, their findings riding back on the result. Human identities skip all of it.

**After** — the same flow, rendered inline as a diagram:

> ```mermaid
> graph TD
>     W[comment / reply write] --> T{author type?}
>     T -- human --> H[lands, no checks]
>     T -- agent --> R{reject-tier finding?}
>     R -- yes --> X[refused: line + fix]
>     R -- no --> L[lands]
>     L --> N[warn-tier notes on the result]
> ```

Reach for a diagram when the reader would otherwise hold three or more transitions in their head. Below that, prose — two boxes and one arrow is a sentence, not a chart. Mermaid fences render inline in the comment view, and the style gate never rejects them: fenced blocks are skipped by every check.

## Brevity and density are different things

Being asked to be concise is an instruction about word count. It is not an instruction to remove blank lines, collapse a list into a sentence, or drop a table.

Density is what makes a comment slow to read. A comment that is three lines longer but broken into blocks is *shorter* to consume than the same content compressed into a paragraph. Optimising the source at the reader's expense is the wrong trade.

A one-line answer still stays one line. This is about matching shape to content, not about decorating everything.

## Self-containment

Rule 10 covers it: no comment IDs, no acronyms, no invented shorthand, no pointing at another comment or section instead of restating what it said. Formatting and self-containment fail together — a comment that says "the other 12 routes" is both unstructured and unanswerable. Fix both at once by writing the list.
