# quecto-image

What an image is, in one place (#2422).

Every quecto peer that handles an image uses this crate and keeps no copy of
its facts: the agent (UDS `prompt` / `steer` / `follow_up`, the `read` tool,
the image token estimate), `quecto-api` (`/prompt`, `/steer`, `/follow_up`,
WebSocket prompt frames), and later extension / MCP tool results (#2423) and
the TUI (#2425). It is a pure leaf crate: no I/O, only `base64` and `serde`.

## What it owns

- `ImageMime`: the allowlist (`image/png`, `image/jpeg`, `image/gif`,
  `image/webp`) and its wire spelling. `ImageMime::parse_exact` matches the
  lowercase wire spelling exactly; `ImageMime::sniff` names the type a file's
  bytes start with.
- `MAX_IMAGE_BYTES`: 3.75 MiB (3,932,160 bytes) decoded, so the base64 is at
  most 5 MiB and Anthropic's 5 MB limit holds whether it is applied to the
  decoded or the encoded size. `MAX_IMAGES_PER_MESSAGE`: 8.
- Base64: images from outside must be strict standard base64 (padded,
  canonical, no whitespace); `encode` writes it. Reading the header of an
  image already held (`dimensions`) is lenient: padding optional,
  non-canonical trailing bits accepted.
- Header parsing: `dimensions(mime, base64)` reads the pixel size from PNG
  IHDR, JPEG SOFn, the GIF screen descriptor and WebP VP8/VP8L/VP8X, decoding
  only the bytes it reads.

## Admission (an allowlist)

`ImageAttachment::new(payload)` (wire), `ImageAttachment::from_bytes(mime,
bytes)` (a file read) and `validate_images(payloads)` (a message's list)
check, in order, and refuse with the first failure:

1. At most 8 images per message: `too many images: 9; at most 8 per message`.
2. `mimeType` exactly one of the four: `mimeType "image/svg+xml" is not
   allowed; use image/png, image/jpeg, image/gif or image/webp`.
3. At most `MAX_IMAGE_BYTES` decoded (longer base64 is refused before it is
   decoded): `image decodes to more than 3932160 bytes (3.75 MiB)`.
4. Strict standard base64: `data is not valid standard base64`.
5. The bytes start with the type's signature: `data does not start with the
   image/png signature`.
6. The header is readable: `not a readable image/png image`.

A list's refusal names the image: `images[1]: …`. `Display` on a refusal is
the exact text clients see; `Debug` never prints the base64. An admitted
`ImageAttachment` keeps its base64 and the pixel size its header gave, and
serialises back to the wire shape `{"mimeType", "data"}`.

The `test-support` feature exposes `samples`: real minimal PNG, JPEG, GIF and
WebP files for other crates' tests.
