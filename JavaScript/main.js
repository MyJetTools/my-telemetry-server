var main = /** @class */ (function () {
    function main() {
    }
    main.resize = function () {
        var height = window.innerHeight;
        var width = window.innerWidth;
        if (this.windowHeight == height && this.windowWidth == width)
            return;
        this.windowHeight = height;
        this.windowWidth = width;
        var sbHeight = this.statusBarHeight;
        this.layoutElement.setAttribute('style', this.generatePosition(0, 0, width, height - sbHeight));
        this.statusBarElement.setAttribute('style', 'position:absolute; ' + this.generatePosition(0, height - sbHeight, width, sbHeight));
    };
    main.generatePosition = function (left, top, width, height) {
        return 'top:' + top + 'px; left:' + left + 'px; width:' + width + 'px; height:' + height + 'px';
    };
    main.background = function () {
        var _this = this;
        if (!this.body) {
            this.body = document.getElementsByTagName('body')[0];
            this.body.innerHTML = HtmlMain.layout();
            this.layoutElement = document.getElementById('main');
            this.statusBarElement = document.getElementById('status-bar');
        }
        this.resize();
        if (this.requested)
            return;
        $.ajax({ url: '/ui/GetServices', type: 'get' })
            .then(function (result) {
            _this.requested = false;
            _this.layoutElement.innerHTML = HtmlMain.generateServicesList(result);
            HtmlStatusBar.updateOnline();
        }).fail(function () {
            _this.requested = false;
            HtmlStatusBar.updateOffline();
        });
    };
    main.requested = false;
    main.statusBarHeight = 24;
    return main;
}());
var $;
window.setTimeout(function () { return main.background(); }, 300);
window.addEventListener('popstate', function (e) {
    console.log(e);
    var state = e.state;
    if (state.action == 'selectService') {
        var el_1 = document.getElementById('app-' + state.id);
        AppSelector.serviceSelected(el_1);
        return;
    }
    var el = document.getElementById(state.div);
    el.innerHTML = state.content;
});
//# sourceMappingURL=main.js.map