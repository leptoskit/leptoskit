import 'dart:convert';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:wire/wire.dart';
Future<void> main() async {
  final requests = <Map<String, Object?>>[];
  final client = Client(httpClient: MockClient((request) async {
    requests.add({'query': request.url.query, 'body': request.body});
    return http.Response('"ok"', 200, headers: {'content-type': 'application/json'});
  }));
  await client.probe(items: [], labels: {}, note: null, enabled: false, count: 0, text: '');
  await client.probe(items: [], labels: {}, note: null, enabled: false, count: 0, text: '', fallback: const Presence<String?>.present(null));
  await client.probe(items: [], labels: {}, note: null, enabled: false, count: 0, text: '', fallback: const Presence<String?>.present(''));
  await client.probe(items: [null, 'a +&é', ''], labels: {'key +&é': null, 'empty': ''}, note: null, enabled: false, count: 0, text: 'a +&é');
  print(jsonEncode(requests));
}
