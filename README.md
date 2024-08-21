# Top Bid Websocket Client

Example top bid websocket client for the ultra sound relay. Shows demonstrates mainly:

1. how to decode the bytes received.
2. how to keep the websocket alive.
3. how to close websocket when done.

Try to be efficient with your websocket connections. The top bid may update thousands of times a second. Pushing updates to hundreds of superfluous or dangling connections slows down the auction for everyone.
