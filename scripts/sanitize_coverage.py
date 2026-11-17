"""Sanitize a coverage GeoJSON written by `bathy coverage`.

Rings that touch themselves at a vertex (diagonally touching survey cells) are
invalid under OGC rules and can make renderers fill huge wrong areas. This script
repairs each feature with GEOS `make_valid`, keeps only polygonal parts, orients rings
per RFC 7946 and rewrites the file in place.

    uv run --with shapely python scripts/sanitize_coverage.py docs/coverage/coverage.geojson
"""
import json
import sys

from shapely import make_valid, set_precision
from shapely.geometry import MultiPolygon, mapping, shape
from shapely.geometry.polygon import orient


def polygons(geom):
    if geom.geom_type == "Polygon":
        return [geom]
    if hasattr(geom, "geoms"):
        return [p for g in geom.geoms for p in polygons(g)]
    return []


def sanitize(geom):
    fixed = geom if geom.is_valid else make_valid(geom)
    parts = [orient(p, 1.0) for p in polygons(fixed) if not p.is_empty and p.area > 0]
    if not parts:
        return None
    return parts[0] if len(parts) == 1 else MultiPolygon(parts)


def main(path):
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
    features, repaired, dropped = [], 0, 0
    for feat in data["features"]:
        geom = shape(feat["geometry"])
        out = sanitize(geom)
        if out is None:
            dropped += 1
            continue
        repaired += not geom.is_valid
        feat["geometry"] = mapping(set_precision(out, 1e-7))
        features.append(feat)
    data["features"] = features
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, separators=(",", ":"))
    print(f"{len(features)} features written, {repaired} repaired, {dropped} dropped")


if __name__ == "__main__":
    main(sys.argv[1])
