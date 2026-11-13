# Native image elements

`Image` accepts bitmap files and SVG through the same source API. Format detection uses the content, including URLs without a filename extension.

```sanim
let logo = <Image src="https://example.org/logo.svg" width={120pt}/>
add(logo)
wait(1s)
```

Sources can also be ordinary paths relative to the script. Local files participate in the runtime's event-driven dependency watching. A missing or corrupt source produces a diagnostic at the `Image` expression.

Images use the shared layout system. Their intrinsic size follows the browser convention of 96 pixels per inch; one source pixel is 0.75pt. Providing one dimension preserves aspect ratio; providing both chooses an explicit image size. Positions, transforms, opacity, and dimensions accept signals just like other elements.

Bitmap decoding and SVG parsing use the same bounded native cache as normal Shrimply previews and paint textures. SVG drawing replays native vectors onto the destination canvas. It does not rasterize to an intermediate image. Bitmap images retain decoded CPU data and upload directly when drawn on a GPU canvas. CUDA still formats outside Skia's codec set retain the existing FFmpeg decoder.

HTTP and HTTPS responses use Shrimply's shared download cache, bounded body sizes, and a 30-second request timeout. Fresh responses are reused according to `Cache-Control`; later preparation or explicit revalidation uses ETag/Last-Modified conditional requests. Remote servers are not polled, and URL dependencies are not registered as filesystem watches. A prepared animation pins the resource bytes it observed. Reloading after the HTTP cache expires can discover a changed response; the runtime cannot infer a server change before revalidation.

SVG resources are self-contained, matching Shrimply's native SVG resource provider; rendering does not fetch arbitrary linked files or URLs from within the SVG.
