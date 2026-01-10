export default {
    async fetch(request, env) {
        const url = new URL(request.url);
        const path = url.pathname;

        if (path === "/install") {
            return fetch("https://raw.githubusercontent.com/jpmacdonald/apl/main/install.sh");
        }

        // Handle manifest.json (used by install.sh)
        if (path === "/manifest.json") {
            const obj = await env.APL_BUCKET.get("manifest.json");
            if (!obj) return new Response("Not found", { status: 404 });
            return new Response(obj.body, {
                headers: {
                    "Content-Type": "application/json",
                    "Access-Control-Allow-Origin": "*"
                }
            });
        }

        // Handle both /index and /index.sig
        if (path === "/index" || path === "/index.sig") {
            const key = path.slice(1);
            const response = await env.APL_BUCKET.get(key);
            if (!response) return new Response(`${key} not found`, { status: 404 });
            return new Response(response.body, {
                headers: {
                    "Content-Type": path.endsWith(".sig") ? "application/pgp-signature" : "application/octet-stream",
                    "Access-Control-Allow-Origin": "*"
                },
            });
        }

        if (path.startsWith("/cas/") || path.startsWith("/deltas/")) {
            const key = path.slice(1);
            const response = await env.APL_BUCKET.get(key);
            if (!response) return new Response("Artifact not found", { status: 404 });
            return new Response(response.body, {
                headers: {
                    "Content-Type": "application/octet-stream",
                    "Cache-Control": "public, max-age=31536000, immutable"
                },
            });
        }

        return new Response("Welcome to APL. Visit https://github.com/jpmacdonald/apl for docs.", {
            headers: { "Content-Type": "text/plain" },
        });
    }
}
