# quecto-image

Validated image attachments for Quecto's UDS protocol (#2422).

The one place Quecto decides whether an image from outside may enter a
conversation. Used by `quecto-agentic-harness` (UDS `prompt` / `steer` /
`follow_up`) and `quecto-api` (`/prompt`, WebSocket prompt frames), so both
apply the same rules and refuse with the same text. Extension and MCP tool
result images (#2423) and TUI attachments (#2425) are meant to use it too.

## Rules (an allowlist)

Checked in this order; the first that fails is the refusal.

1. At most `MAX_IMAGES_PER_MESSAGE` (8) images per message.
2. `mimeType` is exactly `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
3. At most `MAX_IMAGE_BYTES` (5 MiB) decoded; longer base64 is refused before
   it is decoded.
4. `data` is standard base64: padded, no whitespace or line breaks.
5. The decoded bytes start with the declared type's file signature.

## API

- `ImagePayload` — the wire shape `{"mimeType", "data"}`, unvalidated.
- `ImageAttachment::new(payload)` — admits one image or returns an
  `ImageRefusal`; holding an `ImageAttachment` proves it passed. It
  serialises back to the wire shape.
- `validate_images(payloads)` — admits a message's list or returns an
  `ImagesRefusal` naming the failing index (`images[1]: …`).
- `check_images(&payloads)` — the same decision without taking the payloads.

`Display` on a refusal is the exact error text clients see. `Debug` never
prints the base64.
