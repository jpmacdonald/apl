export default {
    async fetch(request, env) {
        const url = new URL(request.url);
        const path = url.pathname;

        // 1. Install Script
        if (path === "/install") {
            return fetch("https://raw.githubusercontent.com/jpmacdonald/apl/main/install.sh");
        }

        // 2. Metadata Inspection (Text Manifest)
        if (path === "/latest") {
            const obj = await env.APL_BUCKET.get("latest.txt");
            if (!obj) return new Response("Not found", { status: 404 });
            return new Response(obj.body, {
                headers: {
                    "Content-Type": "text/plain",
                    "Access-Control-Allow-Origin": "*"
                }
            });
        }

        // 3. Index 
        if (path === "/index") {
            const response = await env.APL_BUCKET.get("index.bin");
            if (!response) return new Response("Index not found", { status: 404 });
            return new Response(response.body, {
                headers: { "Content-Type": "application/octet-stream" },
            });
        }

        // 4. CAS / Storage
        if (path.startsWith("/cas/") || path.startsWith("/deltas/")) {
            const response = await env.APL_BUCKET.get(path.slice(1));
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
