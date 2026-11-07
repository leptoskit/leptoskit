import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:presence/presence.dart';

/// Runs without `package:test`: the generated package only needs `http` and
/// `web_socket_channel`, so a plain Dart program keeps the fixture small.
Future<void> main() async {
  final deadline = Timer(const Duration(seconds: 30), () {
    stderr.writeln('Dart presence fixture exceeded 30 seconds');
    exit(1);
  });
  try {
    _checkModels();
    _checkDefaultsAndNull();
    await _checkArguments();
    await _checkLiveArguments();
    stdout.writeln('generated Dart presence checks passed');
  } finally {
    deadline.cancel();
  }
}

void _checkModels() {
  final requiredOnly = <String, Object?>{
    'required_text': 'r',
    'required_nullable_text': null,
    'required_nullable_nested': null,
    'required_nullable_alias': null,
  };

  // Required nullable values are required constructor inputs, and explicit
  // null is serialized rather than dropped.
  final constructed = Echo(
    requiredText: 'r',
    requiredNullableText: null,
    requiredNullableNested: null,
    requiredNullableAlias: null,
  );
  _equals(
    constructed.toJson(),
    requiredOnly,
    'required nullable constructor values stay present',
  );

  final absent = Echo.fromJson(requiredOnly);
  _equals(
    absent.toJson(),
    requiredOnly,
    'required nullable fields keep their null values',
  );
  _isAbsent(absent.optionalNullableText, 'optional nullable field');
  _isAbsent(absent.optionalNullableNested, 'optional nullable nested field');
  _isAbsent(absent.optionalNullableAlias, 'optional nullable alias field');
  if (absent.optionalText != null ||
      absent.optionalFlag != null ||
      absent.optionalCount != null ||
      absent.optionalItems != null ||
      absent.optionalLabels != null) {
    throw StateError('absent optional fields must stay absent');
  }

  // The four requiredness × nullability combinations, plus falsy and empty
  // values, round-trip through actual encoded JSON.
  final wire = <String, Object?>{
    'required_text': 'r',
    'required_nullable_text': null,
    'optional_text': '',
    'optional_nullable_text': null,
    'required_nullable_nested': <String, Object?>{
      'value': 'outer',
      'note': null,
    },
    'optional_nullable_nested': <String, Object?>{
      'value': 'inner',
      'note': 'kept',
    },
    'required_nullable_alias': null,
    'optional_nullable_alias': 'alias',
    'optional_flag': false,
    'optional_count': 0,
    'optional_empty_text': '',
    'optional_items': <Object?>[],
    'optional_labels': <String, Object?>{},
    'nullable_items': [
      null,
      {'value': 'item'},
    ],
    'nullable_map': {
      'nil': null,
      'item': {'value': 'map'},
    },
    'null_items': [null],
    'null_map': {'nil': null},
  };
  final all = Echo.fromJson(wire);
  _equals(all.toJson(), wire, 'all presence and falsy values round-trip');
  _isPresent(all.optionalNullableText, 'optional nullable field');
  if (all.optionalNullableText.value != null) {
    throw StateError('an explicit optional null must decode as null');
  }
  _isPresent(all.optionalNullableNested, 'optional nullable nested field');
  _equals(
    all.optionalNullableNested.value?.toJson(),
    wire['optional_nullable_nested'],
    'a nested optional value must decode',
  );
  _isPresent(all.optionalNullableAlias, 'optional nullable alias field');
  if (all.optionalNullableAlias.value != 'alias') {
    throw StateError('a nullable reference alias must decode its value');
  }
  if (all.optionalFlag != false ||
      all.optionalCount != 0 ||
      all.optionalText != '' ||
      all.optionalEmptyText != '' ||
      all.optionalItems!.isNotEmpty ||
      all.optionalLabels!.isNotEmpty) {
    throw StateError('falsy and empty optional values must survive decoding');
  }

  // An explicit null on an optional nullable field stays a key, unlike an
  // absent field.
  final explicitNull = Echo.fromJson(<String, Object?>{
    ...wire,
    'optional_nullable_text': null,
    'optional_nullable_nested': null,
    'optional_nullable_alias': null,
    'nullable_items': null,
    'nullable_map': null,
  });
  _equals(explicitNull.toJson(), <String, Object?>{
    ...wire,
    'optional_nullable_text': null,
    'optional_nullable_nested': null,
    'optional_nullable_alias': null,
    'nullable_items': null,
    'nullable_map': null,
  }, 'explicit optional null is not omitted');

  _throws(() {
    Echo.fromJson(<String, Object?>{
      'required_text': 'r',
      'required_nullable_nested': null,
      'required_nullable_alias': null,
    });
  }, 'missing required nullable field');
  _throws(() {
    Echo.fromJson(<String, Object?>{
      'required_nullable_text': null,
      'required_nullable_nested': null,
      'required_nullable_alias': null,
    });
  }, 'missing required non-nullable field');

  // Channel join arguments are ordinary models, so they share the same
  // requiredness and presence behavior.
  final room = Room.fromJson(<String, Object?>{
    'required_text': 'r',
    'required_nullable_text': null,
    'optional_nullable_text': null,
  });
  _equals(room.toJson(), <String, Object?>{
    'required_text': 'r',
    'required_nullable_text': null,
    'optional_nullable_text': null,
  }, 'channel arguments keep an explicit optional null');
  final roomAbsent = Room.fromJson(<String, Object?>{
    'required_text': 'r',
    'required_nullable_text': null,
  });
  _equals(roomAbsent.toJson(), <String, Object?>{
    'required_text': 'r',
    'required_nullable_text': null,
  }, 'channel arguments omit an absent optional null');
}

Future<void> _checkArguments() async {
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  final jsonBodies = <String>[];
  final formBodies = <String>[];
  final subscription = server.listen((request) async {
    final body = await utf8.decoder.bind(request).join();
    if (request.uri.path == '/api/form_echo') {
      formBodies.add(body);
    } else {
      jsonBodies.add(body);
    }
    request.response
      ..statusCode = 200
      ..headers.contentType = ContentType.json
      ..write('null');
    await request.response.close();
  });
  final client = Client(
    serverUrl: 'http://${server.address.host}:${server.port}',
  );

  try {
    await client.send.call(
      requiredText: 'r',
      requiredNullableText: null,
      requiredNullableNested: null,
      optionalNullableText: const Presence<String?>.present(null),
      optionalFlag: false,
      optionalCount: 0,
      optionalEmptyText: '',
      optionalItems: <String>[],
      optionalLabels: <String, String>{},
    );
    _equals(jsonDecode(jsonBodies.single), <String, Object?>{
      'required_text': 'r',
      'required_nullable_text': null,
      'required_nullable_nested': null,
      'optional_nullable_text': null,
      'optional_flag': false,
      'optional_count': 0,
      'optional_empty_text': '',
      'optional_items': <Object?>[],
      'optional_labels': <String, Object?>{},
    }, 'JSON arguments keep null and falsy values');

    await client.send.call(
      requiredText: 'r',
      requiredNullableText: null,
      requiredNullableNested: null,
    );
    _equals(jsonDecode(jsonBodies.last), <String, Object?>{
      'required_text': 'r',
      'required_nullable_text': null,
      'required_nullable_nested': null,
    }, 'JSON arguments omit absent optional values');

    await client.send.call(
      requiredText: 'r',
      requiredNullableText: 'required value',
      requiredNullableNested: Nested(value: 'nested'),
      optionalNullableText: const Presence<String?>.present('value'),
    );
    _equals(jsonDecode(jsonBodies.last), {
      'required_text': 'r',
      'required_nullable_text': 'required value',
      'required_nullable_nested': {'value': 'nested'},
      'optional_nullable_text': 'value',
    }, 'JSON arguments preserve present values');

    await client.formEcho(
      optionalFlag: false,
      optionalCount: 0,
      optionalEmptyText: '',
    );
    final fields = formBodies.single.split('&').toList()..sort();
    _equals(
      fields,
      <String>[
        'optional_flag=false',
        'optional_count=0',
        'optional_empty_text=',
      ]..sort(),
      'form arguments keep false, zero, and an empty string',
    );
  } finally {
    await subscription.cancel();
    await server.close(force: true);
  }
}

void _isAbsent(Presence<Object?> value, String label) {
  if (value.isPresent) {
    throw StateError('$label should be absent');
  }
}

void _isPresent(Presence<Object?> value, String label) {
  if (!value.isPresent) {
    throw StateError('$label should be present');
  }
}

void _throws(void Function() body, String label) {
  try {
    body();
  } catch (_) {
    return;
  }
  throw StateError('$label did not throw');
}

void _equals(Object? actual, Object? expected, String label) {
  if (!_sameJson(actual, expected)) {
    throw StateError(
      '$label: got ${jsonEncode(actual)}, want ${jsonEncode(expected)}',
    );
  }
}

bool _sameJson(Object? actual, Object? expected) {
  if (actual is Map && expected is Map) {
    if (actual.length != expected.length) {
      return false;
    }
    for (final key in actual.keys) {
      if (!expected.containsKey(key) ||
          !_sameJson(actual[key], expected[key])) {
        return false;
      }
    }
    return true;
  }
  if (actual is List && expected is List) {
    if (actual.length != expected.length) {
      return false;
    }
    for (var index = 0; index < actual.length; index++) {
      if (!_sameJson(actual[index], expected[index])) {
        return false;
      }
    }
    return true;
  }
  return actual == expected;
}

void _checkDefaultsAndNull() {
  _throws(() {
    DefaultRequired.fromJson(<String, Object?>{});
  }, 'a default annotation does not make a required field optional');
  _equals(DefaultRequired(value: 'chosen').toJson(), <String, Object?>{
    'value': 'chosen',
  }, 'required default is explicit');
  _equals(
    OnlyNull.fromJson(<String, Object?>{'required_null': null}).toJson(),
    <String, Object?>{'required_null': null},
    'null-only required value',
  );
  _equals(
    OnlyNull.fromJson(<String, Object?>{
      'required_null': null,
      'optional_null': null,
    }).toJson(),
    <String, Object?>{'required_null': null, 'optional_null': null},
    'null-only optional value retains presence',
  );
  _throws(() {
    OnlyNull.fromJson(<String, Object?>{'required_null': 'wrong'});
  }, 'a null-only field cannot decode a non-null value');
}

Future<void> _checkLiveArguments() async {
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  final sockets = <WebSocket>[];
  final mutations = <Object?>[];
  final joined = StreamController<Map<String, dynamic>>();
  final subscriptions = <Object?>[];
  final subscription = server.listen((request) async {
    final socket = await WebSocketTransformer.upgrade(
      request,
      protocolSelector: (_) => 'leptoskit.live.v1.json',
    );
    sockets.add(socket);
    socket.listen((raw) {
      final frame = jsonDecode(raw as String) as Map<String, dynamic>;
      switch (frame['type']) {
        case 'hello':
          socket.add(
            jsonEncode(<String, Object?>{
              'type': 'welcome',
              'version': 0,
              'connection': 'presence-fixture',
              'heartbeat_seconds': 30,
            }),
          );
        case 'mutate':
          mutations.add(frame['args']);
          socket.add(
            jsonEncode(<String, Object?>{
              'type': 'done',
              'request': frame['request'],
              'version': 0,
              'ok': null,
            }),
          );
        case 'sub':
          subscriptions.add(frame['args']);
          socket.add(
            jsonEncode({
              'type': 'transition',
              'version': 0,
              'updates': [
                {'id': frame['id'], 'ok': 'observed'},
              ],
            }),
          );
        case 'join':
          joined.add(frame);
      }
    });
  });
  final client = Client(
    serverUrl: 'http://${server.address.host}:${server.port}',
  );
  try {
    await client.send
        .live(
          requiredText: 'r',
          requiredNullableText: null,
          requiredNullableNested: null,
          optionalNullableText: const Presence<String?>.present(null),
          optionalFlag: false,
          optionalCount: 0,
          optionalEmptyText: '',
          optionalItems: <String>[],
          optionalLabels: <String, String>{},
        )
        .timeout(const Duration(seconds: 5));
    _equals(mutations.single, <String, Object?>{
      'required_text': 'r',
      'required_nullable_text': null,
      'required_nullable_nested': null,
      'optional_nullable_text': null,
      'optional_flag': false,
      'optional_count': 0,
      'optional_empty_text': '',
      'optional_items': <Object?>[],
      'optional_labels': <String, Object?>{},
    }, 'live mutation keeps explicit null and falsy values');
    await client.send
        .live(
          requiredText: 'r',
          requiredNullableText: null,
          requiredNullableNested: null,
        )
        .timeout(const Duration(seconds: 5));
    _equals(mutations.last, <String, Object?>{
      'required_text': 'r',
      'required_nullable_text': null,
      'required_nullable_nested': null,
    }, 'live mutation omits absent values');
    await client.send
        .live(
          requiredText: 'r',
          requiredNullableText: 'text',
          requiredNullableNested: Nested(value: 'nested'),
          optionalNullableText: const Presence<String?>.present('value'),
        )
        .timeout(const Duration(seconds: 5));
    _equals(mutations.last, {
      'required_text': 'r',
      'required_nullable_text': 'text',
      'required_nullable_nested': {'value': 'nested'},
      'optional_nullable_text': 'value',
    }, 'live mutation preserves present values');
    for (final value in [
      const Presence<String?>.absent(),
      const Presence<String?>.present(null),
      const Presence<String?>.present('value'),
    ]) {
      await client.observe
          .watch(
            requiredText: 'r',
            requiredNullableText: null,
            requiredNullableNested: null,
            optionalNullableText: value,
          )
          .first
          .timeout(const Duration(seconds: 5));
      _equals(subscriptions.last, {
        'required_text': 'r',
        'required_nullable_text': null,
        'required_nullable_nested': null,
        if (value.isPresent) 'optional_nullable_text': value.value,
      }, 'subscription preserves presence');
    }
    final joinFrames = StreamIterator(joined.stream);
    for (final value in [
      const Presence<String?>.absent(),
      const Presence<String?>.present(null),
      const Presence<String?>.present('value'),
    ]) {
      final room = client.room.join(
        Room(
          requiredText: 'r',
          requiredNullableText: null,
          optionalNullableText: value,
        ),
      );
      await joinFrames.moveNext().timeout(const Duration(seconds: 5));
      final frame = joinFrames.current;
      _equals(frame['args'], <String, Object?>{
        'required_text': 'r',
        'required_nullable_text': null,
        if (value.isPresent) 'optional_nullable_text': value.value,
      }, 'channel join preserves explicit nullable arguments');
      room.leave();
    }
    final nullRoom = client.nullRoom.join(null);
    await joinFrames.moveNext().timeout(const Duration(seconds: 5));
    final nullFrame = joinFrames.current;
    if (!nullFrame.containsKey('args') || nullFrame['args'] != null) {
      throw StateError('null-only channel args must remain explicit null');
    }
    nullRoom.leave();
    await joinFrames.cancel();
  } finally {
    client.live.close();
    for (final socket in sockets) {
      await socket.close();
    }
    await subscription.cancel();
    await server.close(force: true);
  }
}
