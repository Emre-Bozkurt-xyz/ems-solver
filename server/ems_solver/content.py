"""Validate request parts and turn them into Claude content blocks."""

import base64
import binascii

from .config import Settings
from .errors import ApiError
from .schemas import ImagePart, Part, TextPart

_SIGNATURES = {
    "image/png": (b"\x89PNG\r\n\x1a\n",),
    "image/jpeg": (b"\xff\xd8\xff",),
    "image/gif": (b"GIF87a", b"GIF89a"),
}


def _invalid(message: str) -> ApiError:
    return ApiError(400, "invalid_request", message)


def _matches_media_type(data: bytes, media_type: str) -> bool:
    if media_type == "image/webp":
        return data[:4] == b"RIFF" and data[8:12] == b"WEBP"
    return data.startswith(_SIGNATURES[media_type])


def _image_block(index: int, part: ImagePart, settings: Settings) -> dict:
    # Cheap pre-check so a huge string isn't decoded only to be rejected.
    if len(part.data) > (settings.max_image_bytes * 4) // 3 + 4:
        raise ApiError(413, "payload_too_large", f"parts[{index}]: image exceeds {settings.max_image_bytes} bytes")
    try:
        raw = base64.b64decode(part.data, validate=True)
    except (binascii.Error, ValueError):
        raise _invalid(f"parts[{index}]: image data is not valid base64") from None
    if len(raw) > settings.max_image_bytes:
        raise ApiError(413, "payload_too_large", f"parts[{index}]: image exceeds {settings.max_image_bytes} bytes")
    if not _matches_media_type(raw, part.media_type):
        raise _invalid(f"parts[{index}]: image data does not match media_type {part.media_type}")
    return {
        "type": "image",
        "source": {"type": "base64", "media_type": part.media_type, "data": part.data},
    }


def to_claude_content(parts: list[Part], settings: Settings) -> list[dict]:
    if len(parts) > settings.max_parts:
        raise _invalid(f"too many parts ({len(parts)} > {settings.max_parts})")

    images = sum(isinstance(p, ImagePart) for p in parts)
    if images > settings.max_images:
        raise _invalid(f"too many images ({images} > {settings.max_images})")

    text_chars = sum(len(p.text) for p in parts if isinstance(p, TextPart))
    if text_chars > settings.max_text_chars:
        raise ApiError(413, "payload_too_large", f"text exceeds {settings.max_text_chars} characters")

    blocks = []
    for index, part in enumerate(parts):
        if isinstance(part, TextPart):
            if not part.text.strip():
                raise _invalid(f"parts[{index}]: text is blank")
            blocks.append({"type": "text", "text": part.text})
        else:
            blocks.append(_image_block(index, part, settings))
    return blocks
