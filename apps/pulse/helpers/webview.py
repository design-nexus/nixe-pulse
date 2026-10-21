"""Isolated GTK3/WebKit process. Pulse remains GTK4; identity URLs never enter argv."""
import json, os, re, secrets, sys, threading
from urllib.parse import urlparse
import gi
gi.require_version('Gtk', '3.0')
gi.require_version('WebKit2', '4.1')
from gi.repository import Gtk, WebKit2, GLib, Gio

MAX_MESSAGE = 1024 * 1024

def main():
    config = json.loads(sys.stdin.readline(MAX_MESSAGE))
    url = config['url']
    parsed = urlparse(url)
    if parsed.scheme != 'https' or not parsed.netloc:
        raise ValueError('Only HTTPS apps can be embedded')
    origin = f'{parsed.scheme}://{parsed.netloc}'
    secret = secrets.token_hex(32)
    manager = WebKit2.UserContentManager()
    if not manager.register_script_message_handler('pulse'):
        raise RuntimeError('Could not register Mini App bridge')
    script = '''(() => { if (location.origin !== ORIGIN) return;
      const send = (event, data) => window.webkit.messageHandlers.pulse.postMessage(JSON.stringify({secret:SECRET,event,data}));
      window.TelegramWebviewProxy = {postEvent:(event,data)=>{let parsed={};try{parsed=JSON.parse(data||'{}')}catch(e){}send(event,parsed)}};
      window.external = window.external || {}; window.external.notify = (data)=>{try{let p=JSON.parse(data);send(p.eventType,p.eventData||{})}catch(e){}};
    })();'''.replace('ORIGIN', json.dumps(origin)).replace('SECRET', json.dumps(secret))
    manager.add_script(WebKit2.UserScript.new(script, WebKit2.UserContentInjectedFrames.TOP_FRAME, WebKit2.UserScriptInjectionTime.START, None, None))
    context = WebKit2.WebContext.new_ephemeral()
    view = WebKit2.WebView(web_context=context, user_content_manager=manager)
    settings = view.get_settings()
    settings.set_enable_developer_extras(False)
    settings.set_enable_write_console_messages_to_stdout(False)
    settings.set_javascript_can_access_clipboard(False)
    window = Gtk.Window(title=config.get('title', 'Pulse Mini App'))
    window.set_default_size(460, 720)
    root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
    back = Gtk.Button(label='Back'); back.set_no_show_all(True)
    root.pack_start(back, False, False, 0)
    root.pack_start(view, True, True, 0)
    main_button = Gtk.Button(); main_button.set_no_show_all(True)
    secondary = Gtk.Button(); secondary.set_no_show_all(True)
    root.pack_start(main_button, False, False, 0)
    root.pack_start(secondary, False, False, 0)
    window.add(root)
    state = {'closing':False, 'closed':False}
    def output(event, data=None):
        print(json.dumps({'event':event, 'data':data or {}}), flush=True)
    def emit(event, data=None):
        if not state['closed']:
            js = 'window.Telegram && Telegram.WebView && Telegram.WebView.receiveEvent(%s,%s)' % (json.dumps(event), json.dumps(data or {}))
            view.run_javascript(js, None, None, None)
    def confirm(title, text):
        dialog = Gtk.MessageDialog(transient_for=window, modal=True, message_type=Gtk.MessageType.QUESTION, buttons=Gtk.ButtonsType.YES_NO, text=title)
        dialog.format_secondary_text(text)
        answer = dialog.run() == Gtk.ResponseType.YES
        dialog.destroy()
        return answer
    def external(url):
        if urlparse(url).scheme not in ('https','http'):
            return
        if confirm('Open external website?', url):
            Gio.AppInfo.launch_default_for_uri(url, None)
    def viewport(*_):
        emit('viewport_changed', {'height':view.get_allocated_height(), 'is_state_stable':True, 'is_expanded':True})
        emit('safe_area_changed',dict(top=0,bottom=0,left=0,right=0))
        emit('content_safe_area_changed',dict(top=0,bottom=0,left=0,right=0))
    def dispatch(event, data):
        if event in ('web_app_ready','web_app_request_theme'):
            emit('theme_changed', {'theme_params':config.get('theme',{})}); viewport()
        elif event in ('web_app_request_viewport','web_app_request_safe_area','web_app_request_content_safe_area'):
            viewport()
        elif event == 'web_app_close':
            window.close()
        elif event == 'web_app_expand':
            window.resize(600,850)
        elif event == 'web_app_request_fullscreen':
            window.fullscreen(); emit('fullscreen_changed', {'is_fullscreen':True})
        elif event == 'web_app_exit_fullscreen':
            window.unfullscreen(); emit('fullscreen_changed', {'is_fullscreen':False})
        elif event in ('web_app_setup_main_button','web_app_setup_secondary_button'):
            button = main_button if event.endswith('main_button') else secondary
            button.set_label(str(data.get('text','Continue'))[:128]); button.set_sensitive(bool(data.get('is_active',True)))
            button.set_visible(bool(data.get('is_visible',False)))
        elif event == 'web_app_setup_back_button':
            back.set_visible(bool(data.get('is_visible',False)))
        elif event == 'web_app_setup_closing_behavior':
            state['closing'] = bool(data.get('need_confirmation',False))
        elif event == 'web_app_open_link':
            external(str(data.get('url','')))
        elif event == 'web_app_open_popup':
            dialog = Gtk.MessageDialog(transient_for=window,modal=True,message_type=Gtk.MessageType.INFO,buttons=Gtk.ButtonsType.NONE,text=str(data.get('title','Mini App'))[:128])
            dialog.format_secondary_text(str(data.get('message',''))[:4096])
            buttons = data.get('buttons') or [{'id':'','type':'ok','text':'OK'}]
            for i,button in enumerate(buttons[:3]): dialog.add_button(str(button.get('text') or button.get('type','OK'))[:64],i)
            result=dialog.run(); dialog.destroy()
            emit('popup_closed',{'button_id':buttons[result].get('id','') if 0<=result<min(3,len(buttons)) else None})
        elif event in ('web_app_set_background_color','web_app_set_header_color','web_app_set_bottom_bar_color'):
            color=data.get('color','')
            if re.fullmatch(r'#[0-9a-fA-F]{6}',color):
                css=Gtk.CssProvider();css.load_from_data(('window {background-color:'+color+';}').encode());window.get_style_context().add_provider(css,Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        elif event == 'web_app_request_file_download':
            uri=str(data.get('url',''))
            if urlparse(uri).scheme!='https':
                emit('file_download_requested',{'status':'cancelled'});return
            chooser=Gtk.FileChooserDialog(title='Download Mini App file',transient_for=window,action=Gtk.FileChooserAction.SAVE)
            chooser.add_buttons('Cancel',Gtk.ResponseType.CANCEL,'Download',Gtk.ResponseType.ACCEPT)
            chooser.set_do_overwrite_confirmation(True)
            chooser.set_current_name(os.path.basename(str(data.get('file_name','download')))[:255] or 'download')
            answer=chooser.run();path=chooser.get_filename();chooser.destroy()
            if answer!=Gtk.ResponseType.ACCEPT or not path:
                emit('file_download_requested',{'status':'cancelled'});return
            download=view.download_uri(uri)
            download.set_destination(Gio.File.new_for_path(path).get_uri())
            download.set_allow_overwrite(True)
            download.connect('failed',lambda *_:emit('file_download_requested',{'status':'cancelled'}))
            emit('file_download_requested',{'status':'downloading'})
        elif event == 'web_app_trigger_haptic_feedback':
            pass
        else: output(event,data)
    def received(_manager,result):
        try:
            raw=result.get_js_value().to_string()
            if len(raw)>MAX_MESSAGE:return
            payload=json.loads(raw)
            if payload.get('secret')!=secret:return
            if f'{urlparse(view.get_uri() or url).scheme}://{urlparse(view.get_uri() or url).netloc}' != origin:return
            dispatch(payload.get('event',''),payload.get('data') or {})
        except (ValueError,TypeError,AttributeError): pass
    manager.connect('script-message-received::pulse', received)
    def policy(_view,decision,kind):
        if kind == WebKit2.PolicyDecisionType.NAVIGATION_ACTION:
            uri=decision.get_navigation_action().get_request().get_uri()
            p=urlparse(uri)
            if uri=='about:blank':return False
            if p.scheme=='tg':decision.ignore();output('web_app_open_tg_link',{'url':uri});return True
            if f'{p.scheme}://{p.netloc}'!=origin:
                decision.ignore();external(uri);return True
        elif kind == WebKit2.PolicyDecisionType.NEW_WINDOW_ACTION:
            decision.ignore();external(decision.get_navigation_action().get_request().get_uri());return True
        return False
    view.connect('decide-policy',policy)
    def permission(_view,request):
        # Geolocation is routed through the native bridge, never implicitly granted.
        if isinstance(request,WebKit2.UserMediaPermissionRequest):
            requested=[]
            if request.get_property("is-for-audio-device"):requested.append('microphone')
            if request.get_property("is-for-video-device"):requested.append('camera')
            if confirm('Allow device access?',origin+' requests your '+', '.join(requested)+'.'):
                request.allow();return True
        request.deny();return True
    view.connect('permission-request',permission)
    def load_failed(_view,_event,_uri,error):
        output('bridge_error',{'message':str(error)});return False
    view.connect('load-failed',load_failed)
    view.connect('size-allocate',viewport)
    main_button.connect('clicked',lambda *_:emit('main_button_pressed'))
    secondary.connect('clicked',lambda *_:emit('secondary_button_pressed'))
    back.connect('clicked',lambda *_:emit('back_button_pressed'))
    def closing(*_):
        if state['closing'] and not confirm('Close Mini App?','Unsaved changes may be lost.'):return True
        return False
    def destroyed(*_):
        state['closed']=True;output('closed');Gtk.main_quit()
    window.connect('delete-event',closing);window.connect('destroy',destroyed)
    def incoming():
        for line in sys.stdin:
            if len(line)>MAX_MESSAGE:continue
            try: command=json.loads(line)
            except ValueError:continue
            def apply(command=command):
                if command.get('close'):window.destroy()
                else:emit(command.get('event',''),command.get('data',{}))
                return False
            GLib.idle_add(apply)
        GLib.idle_add(window.destroy)
    threading.Thread(target=incoming,daemon=True).start()
    window.show_all()
    if config.get("html"):
        view.load_html(config["html"],url)
    else:
        view.load_uri(url)
    Gtk.main()

if __name__=='__main__':
    try: main()
    except Exception as error:
        print(json.dumps({'event':'bridge_error','data':{'message':str(error)}}),flush=True)
        sys.exit(1)
