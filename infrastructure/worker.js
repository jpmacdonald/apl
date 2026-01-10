export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const key = url.pathname.slice(1); // Remove leading slash

    // Backward compatibility: Map manifest.json to latest.json
    let objectKey = key;
    if (key === 'manifest.json' || key === 'latest' || key === 'latest.json') {
      objectKey = 'latest.json';
    }

    // Allow index, index.sig, and latest.json
    if (!['index', 'index.sig', 'latest.json'].includes(objectKey)) {
      return new Response('Not Found', { status: 404 });
    }

    const object = await env.APL_BUCKET.get(objectKey);

    if (object === null) {
      return new Response('Object Not Found', { status: 404 });
    }

    const headers = new Headers();
    object.writeHttpMetadata(headers);
    headers.set('etag', object.httpEtag);
    headers.set('Cache-Control', 'public, max-age=60'); // Short cache for manifest/index

    return new Response(object.body, {
      headers,
    });
  },
};
