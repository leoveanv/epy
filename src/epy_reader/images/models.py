"""Image source data and the helper's explicit rendering request."""

from typing import Optional, Tuple

from pydantic import BaseModel, Field


class ImageSource(BaseModel):
    name: str
    data: bytes


class ImageTarget(BaseModel):
    row: int
    path: str


class ImagePlacement(BaseModel):
    rows: int = Field(gt=0)
    cols: int = Field(gt=0)
    top: int = Field(default=0, ge=0)
    left: int = Field(default=0, ge=0)


class ImageCells(BaseModel):
    transmission: str
    lines: Tuple[str, ...]


class ImageRequest(BaseModel):
    rows: int = Field(gt=0)
    cols: int = Field(gt=0)
    image_id: int = Field(gt=0, le=0xFFFFFFFF)
    cell_width_px: int = Field(default=8, gt=0)
    cell_height_px: int = Field(default=16, gt=0)
    in_tmux: bool = False
    placement: Optional[ImagePlacement] = None


class GraphicsSupport(BaseModel):
    in_tmux: bool = False
    reason: Optional[str] = None
    cell_width_px: int = Field(default=8, gt=0)
    cell_height_px: int = Field(default=16, gt=0)
