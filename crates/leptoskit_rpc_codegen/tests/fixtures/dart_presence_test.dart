import 'dart:convert';
import 'dart:io';

import 'package:presence/presence.dart';

/// Runs without `package:test`: the generated package only needs `http` and
/// `web_socket_channel`, so a plain Dart program keeps the fixture small.
Future<void> main() async {
  _checkModels();
  await _checkArguments();
  stdout.writeln('generated Dart presence checks passed');
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
  });
  _equals(
    explicitNull.toJson(),
    <String, Object?>{
      ...wire,
      'optional_nullable_text': null,
      'optional_nullable_nested': null,
      'optional_nullable_alias': null,
    },
    'explicit optional null is not omitted',
  );

  _throws(
    () {
      Echo.fromJson(<String, Object?>{
        'required_text': 'r',
        'required_nullable_nested': null,
        'required_nullable_alias': null,
      });
    },
    'missing required nullable field',
  );
  _throws(
    () {
      Echo.fromJson(<String, Object?>{
        'required_nullable_text': null,
        'required_nullable_nested': null,
        'required_nullable_alias': null,
      });
    },
    'missing required non-nullable field',
  );

  // Channel join arguments are ordinary models, so they share the same
  // requiredness and presence behavior.
  final room = Room.fromJson(<String, Object?>{
    'required_text': 'r',
    'required_nullable_text': null,
    'optional_nullable_text': null,
  });
  _equals(
    room.toJson(),
    <String, Object?>{
      'required_text': 'r',
      'required_nullable_text': null,
      'optional_nullable_text': null,
    },
    'channel arguments keep an explicit optional null',
  );
  final roomAbsent = Room.fromJson(<String, Object?>{
    'required_text': 'r',
    'required_nullable_text': null,
  });
  _equals(
    roomAbsent.toJson(),
    <String, Object?>{
      'required_text': 'r',
      'required_nullable_text': null,
    },
    'channel arguments omit an absent optional null',
  );
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
    _equals(
      jsonDecode(jsonBodies.single),
      <String, Object?>{
        'required_text': 'r',
        'required_nullable_text': null,
        'required_nullable_nested': null,
        'optional_nullable_text': null,
        'optional_flag': false,
        'optional_count': 0,
        'optional_empty_text': '',
        'optional_items': <Object?>[],
        'optional_labels': <String, Object?>{},
      },
      'JSON arguments keep null and falsy values',
    );

    await client.send.call(
      requiredText: 'r',
      requiredNullableText: null,
      requiredNullableNested: null,
    );
    _equals(
      jsonDecode(jsonBodies.last),
      <String, Object?>{
        'required_text': 'r',
        'required_nullable_text': null,
        'required_nullable_nested': null,
      },
      'JSON arguments omit absent optional values',
    );

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
