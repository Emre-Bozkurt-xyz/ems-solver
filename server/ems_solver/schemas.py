from typing import Annotated, Literal, Union

from pydantic import BaseModel, ConfigDict, Field

ImageMediaType = Literal["image/png", "image/jpeg", "image/gif", "image/webp"]
Mode = Literal["solution", "hint", "checkwork"]


class TextPart(BaseModel):
    model_config = ConfigDict(extra="forbid")
    type: Literal["text"]
    text: str = Field(min_length=1)


class ImagePart(BaseModel):
    model_config = ConfigDict(extra="forbid")
    type: Literal["image"]
    media_type: ImageMediaType
    data: str = Field(min_length=1, description="Base64, no data: prefix")


Part = Annotated[Union[TextPart, ImagePart], Field(discriminator="type")]


class SolveRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    mode: Mode = "solution"
    parts: list[Part] = Field(min_length=1)
