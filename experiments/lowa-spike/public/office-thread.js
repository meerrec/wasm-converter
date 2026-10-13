/* Харнесс спайка LOWA: сторона офиса (внутри pthread-воркера Emscripten).
 *
 * Файл приезжает сюда не как <script> страницы, а через Module.uno_scripts: C++-сторона
 * делает importScripts(url) уже в том воркере, где живёт офис (runtime-contract §1-2).
 * Поэтому здесь есть UNO и нет document.
 *
 * Протокол к странице (порт — Module.uno_main у страницы, Module.uno_mainPort здесь,
 * это две стороны одного MessageChannel):
 *   страница -> офис: {id, cmd:'ping'|'probe'|'convert', ...}
 *   офис -> страница: {id, ok:true, ...} | {id, ok:false, error:{message, type, stack}} | {cmd:'ready'}
 */
'use strict';

(function () {
  var PROTOCOL = 'lowa-harness/1';

  var zetajs = null;
  var css = null;
  var context = null;
  var desktop = null;
  var xModel = null;

  /** Раскладывает UNO-исключение в плоский объект: сам объект исключения через порт не пройдёт. */
  function unoError(e) {
    var info = {
      message: e && e.message ? String(e.message) : String(e),
      type: null,
      stack: e && e.stack ? String(e.stack).split('\n').slice(0, 6).join('\n') : null,
    };
    try {
      var exc = zetajs.catchUnoException(e);
      if (exc !== undefined && exc !== null) {
        if (exc.Message !== undefined) {
          info.message = String(exc.Message);
        }
        try {
          var t = zetajs.getAnyType(exc);
          if (t !== undefined && t !== null) {
            info.type = String(t);
          }
        } catch (ignored) {
          /* тип недоступен — не повод терять сообщение */
        }
      }
    } catch (ignored) {
      /* не UNO-исключение: остаётся message из JS */
    }
    return info;
  }

  /** Приводит UNO-последовательность к массиву: zetajs отдаёт либо JS-массив, либо seq-объект. */
  function toArray(value) {
    if (Array.isArray(value)) {
      return value;
    }
    if (value && typeof value.size === 'function' && typeof value.get === 'function') {
      var out = [];
      for (var i = 0; i < value.size(); ++i) {
        out.push(value.get(i));
      }
      return out;
    }
    return [];
  }

  /** Значение свойства фильтра бывает строкой, числом, boolean или enum — берём то, что читается. */
  function plainValue(value) {
    if (value === null || value === undefined) {
      return null;
    }
    var t = typeof value;
    if (t === 'string' || t === 'number' || t === 'boolean') {
      return value;
    }
    return String(value);
  }

  /** Свойства одной записи FilterFactory: массив PropertyValue -> объект {имя: значение}. */
  function propsToObject(seq) {
    var out = {};
    toArray(seq).forEach(function (pv) {
      if (pv && pv.Name !== undefined) {
        out[String(pv.Name)] = plainValue(pv.Value);
      }
    });
    return out;
  }

  /** Возвращает синглтон FilterFactory — zetajs отдаёт его то функцией, то сервисом с .create(ctx). */
  function filterFactory() {
    var member = css.document.FilterFactory;
    if (typeof member !== 'function') {
      return member;
    }
    if (typeof member.create === 'function') {
      return member.create(context);
    }
    return member(context);
  }

  /** Приводит объект к XNameAccess: у прокси zeta.js методы всех интерфейсов уже на месте. */
  function asNameAccess(object) {
    if (object && typeof object.getElementNames === 'function') {
      return object;
    }
    var raw = css.container.XNameAccess;
    if (raw && typeof raw.query === 'function') {
      return raw.query(object);
    }
    throw new Error('объект не похож на XNameAccess');
  }

  /** Пробует разные дороги к FilterFactory и записывает, какая сработала. */
  function locateFilterFactory(diag) {
    var routes = [
      ['css.document.FilterFactory', function () {
        return filterFactory();
      }],
      ['context.getByName(/singletons/...)', function () {
        return context.getByName('/singletons/com.sun.star.document.FilterFactory');
      }],
      ['serviceManager.createInstanceWithContext', function () {
        return context
          .getServiceManager()
          .createInstanceWithContext('com.sun.star.document.FilterFactory', context);
      }],
    ];
    for (var i = 0; i < routes.length; ++i) {
      var name = routes[i][0];
      try {
        var found = routes[i][1]();
        var kind = found === null || found === undefined ? String(found) : typeof found;
        var keys = found ? Object.keys(found).slice(0, 25) : [];
        var ok = found && typeof found.getElementNames === 'function';
        diag[name] = { type: kind, hasGetElementNames: !!ok, keys: keys };
        if (ok) {
          diag.winner = name;
          return found;
        }
      } catch (e) {
        diag[name] = { error: unoError(e).message };
      }
    }
    return null;
  }

  /** Перечисляет фильтры и, для pdf-подобных, вытаскивает их свойства из FilterFactory. */
  function probe() {
    var diag = { moduleDocumentKeys: Object.keys(css.document).slice(0, 30) };
    var factory = locateFilterFactory(diag);
    if (factory === null) {
      return { count: 0, names: [], detailed: {}, errors: [], diag: diag };
    }
    var names = toArray(asNameAccess(factory).getElementNames()).map(String);
    var detailed = {};
    var errors = [];
    names.forEach(function (name) {
      if (!/pdf/i.test(name)) {
        return;
      }
      try {
        detailed[name] = propsToObject(factory.getByName(name));
      } catch (e) {
        errors.push({ name: name, error: unoError(e) });
      }
    });
    return { count: names.length, names: names, detailed: detailed, errors: errors, diag: diag };
  }

  /** Закрывает модель предыдущей конвертации: офис иначе держит документ открытым. */
  function closeModel() {
    if (xModel === null) {
      return;
    }
    var model = xModel;
    xModel = null;
    try {
      if (model.queryInterface(zetajs.type.interface(css.util.XCloseable))) {
        model.close(false);
      }
    } catch (e) {
      /* закрыть не удалось — не повод ронять конвертацию, следующий loadComponentFromURL
         откроет документ рядом */
    }
  }

  /** FilterData заполняем целиком: незаданные опции наследуют прошлые настройки экспорта. */
  function exportArgs(filter, extra) {
    var args = [
      new css.beans.PropertyValue({ Name: 'FilterName', Value: filter }),
      new css.beans.PropertyValue({ Name: 'Overwrite', Value: true }),
    ];
    toArray(extra).forEach(function (item) {
      if (item && item.Name !== undefined) {
        args.push(new css.beans.PropertyValue({ Name: String(item.Name), Value: item.Value }));
      }
    });
    return args;
  }

  function convert(msg) {
    closeModel();
    var loadArgs = [
      new css.beans.PropertyValue({ Name: 'Hidden', Value: true }),
      new css.beans.PropertyValue({ Name: 'ReadOnly', Value: true }),
    ];
    xModel = desktop.loadComponentFromURL('file://' + msg.from, '_default', 0, loadArgs);
    if (xModel === null || xModel === undefined) {
      throw new Error('loadComponentFromURL returned null for ' + msg.from);
    }
    xModel.storeToURL('file://' + msg.to, exportArgs(msg.filter, msg.filterData));
    return { from: msg.from, to: msg.to, filter: msg.filter };
  }

  function handle(port, msg) {
    var started = Date.now();
    try {
      var payload;
      switch (msg.cmd) {
        case 'ping':
          payload = { state: xModel === null ? 'idle' : 'document-open' };
          break;
        case 'probe':
          payload = probe();
          break;
        case 'convert':
          payload = convert(msg);
          break;
        default:
          throw new Error('unknown command: ' + msg.cmd);
      }
      payload.ms = Date.now() - started;
      payload.ok = true;
      payload.id = msg.id;
      payload.protocol = PROTOCOL;
      port.postMessage(payload);
    } catch (e) {
      port.postMessage({
        protocol: PROTOCOL,
        id: msg.id,
        ok: false,
        ms: Date.now() - started,
        error: unoError(e),
      });
    }
  }

  function start(zj) {
    zetajs = zj;
    css = zetajs.uno.com.sun.star;
    context = zetajs.getUnoComponentContext();
    desktop = css.frame.Desktop.create(context);
    var port = zetajs.mainPort;
    port.onmessage = function (e) {
      handle(port, e.data);
    };
    port.postMessage({ protocol: PROTOCOL, cmd: 'ready', filterCount: -1 });
  }

  var Module = globalThis.Module;
  if (Module === undefined || Module.zetajs === undefined) {
    throw new Error('office-thread.js: Module.zetajs is missing — zeta.js must be uno_scripts[0]');
  }
  Module.zetajs.then(start, function (e) {
    console.error('office-thread.js: zetajs failed to initialize', e);
  });
})();
