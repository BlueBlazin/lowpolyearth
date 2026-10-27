// Answers the page's "clear all data" request. The response header makes the
// browser remove this site's HTTP cache and storage; the page stops the app
// before asking.
export function onRequestPost() {
    return new Response(null, {
        status: 204,
        headers: {
            "Clear-Site-Data": '"cache", "storage"',
            "Cache-Control": "no-store",
        },
    });
}
