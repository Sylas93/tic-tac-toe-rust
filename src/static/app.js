const { webSocket } = rxjs.webSocket;
var socketConnection;
var figure = "";
var gameState = "NO_GAME";
var standardBackgroundColor = "#aa88b9";
var endFlag = false;

function initialize() {
    if (gameState === "IN_GAME") return;

    if (gameState === "NO_GAME") {
        initializeCells();
    } else if (gameState === "END_GAME") {
        resetCells();
    }
    $("h2").css("background", standardBackgroundColor);
    gameState = "IN_GAME";
    endFlag = false;
    connectSocket();
}

function connectSocket() {
    socketConnection = webSocket("ws://127.0.0.1:8080/socket");
    socketConnection.subscribe({
        next: msg => handleNext(msg), // Called whenever there is a message from the server.
        error: err => handleError(err), // Called if at any point WebSocket API signals some kind of error.
        complete: () => handleComplete() // Called when connection is closed (for whatever reason).
    });
}

function clickImageHandler(clicked_id) {
    console.log("Clicked " + clicked_id)
    socketConnection.next(createMessage(clicked_id, "CLIENT_CLICK"));
}

function createMessage(text, type) {
    return { "text": text, "type": type };
}

function handleNext(msg) {
    console.log(msg);
    if (msg.type === "FIGURE") {
        figure = msg.text;
    } else if (msg.type === "SHOW") {
        $(`#${msg.text} .img-responsive`).attr("src", `images/${figure}.jpg`);
    } else if (msg.type === "INFO") {
        console.log("Got an info!")
        $("h2").html(msg.text);
    } else if (msg.type === "END") {
        console.log("Game end!")
        endFlag = true;
        $("h2").html(msg.text);
        $("h2").css("background", "darkseagreen");
        delayedEndGame();
    }
}

function handleError(err) {
    console.log(err);
    if (endFlag) return;
    $("h2").html("Ops, connection lost<br>:(<br>Tap here to play again!");
    $("h2").css("background", "indianred");
    delayedEndGame();
}

function handleComplete() {
    console.log('complete');
}

function delayedEndGame() {
    setTimeout(() => {
        gameState = "END_GAME"
    }, 1400);
}

function initializeCells() {
    $(".container").append(`
        <div class="row mb-3" style="padding-right: 15px; padding-left: 15px;"></div>
    `)
    for (let i = 0; i < 9; i++) {
        $(".row").append(`
            <div id=${i} onClick="clickImageHandler(this.id)" class="col-xs-4 themed-grid-col">
                <img src="images/empty-cell.jpg" class="img-responsive center-block">
            </div>
        `)
    }
}

function resetCells() {
    for (let i = 0; i < 9; i++) {
        $(`#${i} .img-responsive`).attr("src", `images/empty-cell.jpg`);
    }
}
