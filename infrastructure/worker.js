export default {
    async fetch(request, env) {
        const url = new URL(request.url);
        const path = url.pathname;

        if (path === "/install") {
            return fetch("https://raw.githubusercontent.com/jpmacdonald/apl/main/install.sh");
        }

        // Handle latest.json (used by install.sh)
        if (path === "/latest.json") {
            const obj = await env.APL_BUCKET.get("latest.json");
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

        if (path.startsWith("/ports/") || path.startsWith("/cas/") || path.startsWith("/deltas/")) {
            const key = path.slice(1);
            const response = await env.APL_BUCKET.get(key);
            if (!response) return new Response("Artifact not found", { status: 404 });
            return new Response(response.body, {
                headers: {
                    "Content-Type": path.endsWith(".json") ? "application/json" : "application/octet-stream",
                    "Cache-Control": path.startsWith("/ports/") ? "public, max-age=60" : "public, max-age=31536000, immutable",
                    "Access-Control-Allow-Origin": "*"
                },
            });
        }

        return new Response("Welcome to APL. Visit https://github.com/jpmacdonald/apl for docs.", {
            headers: { "Content-Type": "text/plain" },
        });
    }
}
