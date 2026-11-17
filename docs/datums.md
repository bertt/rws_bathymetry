# Vertical datums

| Name | Meaning |
|---|---|
| NAP (EPSG:5709) | Dutch national height datum, about mean sea level. Heights in the `*_NAP.tif` files. |
| LAT | Lowest Astronomical Tide. Chart datum used in the `*_LAT.tif` files; not used here. |
| EPSG:7415 | Compound CRS: RD New (EPSG:28992) + NAP height. |
| EPSG:4979 | ETRS89 3D (geographic, ellipsoidal height); equivalent to WGS84 at metre level. |

## Conversion

`h_ellipsoid = H_NAP + N(lon, lat)` where N is the NLGEO2018 quasi-geoid height (about 43.3 m at Amersfoort).
The transformation EPSG:7415 → EPSG:4979 is done with terrio-crs and the PROJ grid `nl_nsgi_nlgeo2018.tif`
(plus `nl_nsgi_rdtrans2018.tif` for the horizontal part). Run `bathy fetch-grids` once. Without the grids the height
component silently stays 0, so the tool always requires them.

Cesium terrain heights are WGS84 ellipsoid heights. This is why the output is ellipsoidal:
3D Tiles and the PDOK terrain use the same reference.

Terrarium output defaults to NAP (what basemap DEMs expect) and can be switched with `--datum ellipsoid`.

## Verification

The GeoTIFFs have no embedded vertical CRS. The NAP reference was verified from the ISO XML abstract ("t.o.v. NAP") and
the converted values (seabed −10 … −3 m NAP became 32 … 40 m ellipsoidal height around IJmuiden).
